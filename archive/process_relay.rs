//! 已归档：Windows 进程级实时音频流转实验。

use std::{
    collections::VecDeque,
    iter, ptr,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;
use windows::{
    Win32::{
        Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_NOPERSIST,
            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
            IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance},
            Threading::WaitForMultipleObjects,
        },
    },
    core::PCWSTR,
};

use super::{
    eq::{ParametricEq, SessionEqConfig},
    process_loopback::{
        CAPTURE_CHANNELS, CAPTURE_SAMPLE_RATE, ComApartment, OwnedEvent,
        activate_process_audio_client, capture_format,
    },
};

const PREBUFFER_FRAMES: usize = CAPTURE_SAMPLE_RATE as usize * 40 / 1_000;
const MAX_QUEUE_FRAMES: usize = CAPTURE_SAMPLE_RATE as usize / 2;
const EQ_CROSSFADE_FRAMES: usize = CAPTURE_SAMPLE_RATE as usize * 20 / 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessRelayStatus {
    pub active: bool,
    pub pid: Option<u32>,
    pub target_device_id: Option<String>,
    pub target_device_name: Option<String>,
    pub elapsed_ms: u64,
    pub last_error: Option<String>,
    pub queued_frames: u32,
    pub buffer_ms: f32,
    pub frames_captured: u64,
    pub frames_rendered: u64,
    pub underrun_frames: u64,
    pub dropped_frames: u64,
    pub playback_rate: f32,
    pub eq_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessRelayResult {
    pub pid: u32,
    pub target_device_id: String,
    pub target_device_name: String,
    pub frames_captured: u64,
    pub frames_rendered: u64,
    pub underrun_frames: u64,
    pub dropped_frames: u64,
    pub duration_ms: u64,
    pub eq_applied: bool,
}

struct ActiveRelay {
    pid: u32,
    target_device_id: String,
    target_device_name: String,
    started_at: Instant,
    stop_tx: Sender<()>,
    eq_tx: Sender<SessionEqConfig>,
    result_rx: Receiver<Result<ProcessRelayResult, String>>,
    join: JoinHandle<()>,
    telemetry: Arc<Mutex<RelayTelemetry>>,
}

#[derive(Default)]
struct RelayManagerState {
    active: Option<ActiveRelay>,
    last_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct ProcessRelayManager {
    state: Arc<Mutex<RelayManagerState>>,
}

#[derive(Debug, Clone)]
struct RelayTelemetry {
    queued_frames: u32,
    frames_captured: u64,
    frames_rendered: u64,
    underrun_frames: u64,
    dropped_frames: u64,
    playback_rate: f32,
    eq_enabled: bool,
}

impl RelayTelemetry {
    fn new(eq_enabled: bool) -> Self {
        Self {
            queued_frames: 0,
            frames_captured: 0,
            frames_rendered: 0,
            underrun_frames: 0,
            dropped_frames: 0,
            playback_rate: 1.0,
            eq_enabled,
        }
    }
}

impl ProcessRelayManager {
    pub fn start(
        &self,
        pid: u32,
        target_device_id: String,
        target_device_name: String,
        eq_config: SessionEqConfig,
    ) -> Result<ProcessRelayStatus, String> {
        if pid == 0 {
            return Err("系统声音暂不支持按进程实时流转".to_string());
        }
        if target_device_id.trim().is_empty() {
            return Err("请选择流转目标设备".to_string());
        }
        eq_config.validate()?;
        self.refresh_finished_relay();
        {
            let state = self
                .state
                .lock()
                .map_err(|_| "流转状态锁已损坏".to_string())?;
            if let Some(active) = &state.active {
                return Err(format!("PID {} 正在流转，请先停止当前流转", active.pid));
            }
        }

        let (stop_tx, stop_rx) = mpsc::channel();
        let (eq_tx, eq_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let telemetry = Arc::new(Mutex::new(RelayTelemetry::new(eq_config.enabled)));
        let thread_telemetry = Arc::clone(&telemetry);
        let thread_device_id = target_device_id.clone();
        let thread_device_name = target_device_name.clone();
        let join = thread::Builder::new()
            .name(format!("audio-hub-relay-{pid}"))
            .spawn(move || {
                let context = RelayThreadContext {
                    pid,
                    target_device_id: thread_device_id,
                    target_device_name: thread_device_name,
                    eq_config,
                    telemetry: thread_telemetry,
                };
                let result = relay_process_audio(context, stop_rx, eq_rx, ready_tx);
                let _ = result_tx.send(result);
            })
            .map_err(|error| format!("无法启动实时流转线程: {error}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "流转状态锁已损坏".to_string())?;
                state.last_error = None;
                state.active = Some(ActiveRelay {
                    pid,
                    target_device_id,
                    target_device_name,
                    started_at: Instant::now(),
                    stop_tx,
                    eq_tx,
                    result_rx,
                    join,
                    telemetry,
                });
                drop(state);
                Ok(self.status())
            }
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = stop_tx.send(());
                let _ = join.join();
                Err("初始化实时音频流转超时".to_string())
            }
        }
    }

    pub fn stop(&self) -> Result<ProcessRelayResult, String> {
        self.refresh_finished_relay();
        let active = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "流转状态锁已损坏".to_string())?;
            state
                .active
                .take()
                .ok_or_else(|| "当前没有正在进行的音频流转".to_string())?
        };
        let _ = active.stop_tx.send(());
        let _ = active.join.join();
        let result = active
            .result_rx
            .recv()
            .map_err(|_| "流转线程未返回结果".to_string())?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| "流转状态锁已损坏".to_string())?;
        state.last_error = result.as_ref().err().cloned();
        result
    }

    pub fn status(&self) -> ProcessRelayStatus {
        self.refresh_finished_relay();
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &state.active {
            Some(active) => {
                let telemetry = active
                    .telemetry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone();
                ProcessRelayStatus {
                    active: true,
                    pid: Some(active.pid),
                    target_device_id: Some(active.target_device_id.clone()),
                    target_device_name: Some(active.target_device_name.clone()),
                    elapsed_ms: active.started_at.elapsed().as_millis() as u64,
                    last_error: state.last_error.clone(),
                    queued_frames: telemetry.queued_frames,
                    buffer_ms: telemetry.queued_frames as f32 * 1_000.0
                        / CAPTURE_SAMPLE_RATE as f32,
                    frames_captured: telemetry.frames_captured,
                    frames_rendered: telemetry.frames_rendered,
                    underrun_frames: telemetry.underrun_frames,
                    dropped_frames: telemetry.dropped_frames,
                    playback_rate: telemetry.playback_rate,
                    eq_enabled: telemetry.eq_enabled,
                }
            }
            None => ProcessRelayStatus {
                active: false,
                pid: None,
                target_device_id: None,
                target_device_name: None,
                elapsed_ms: 0,
                last_error: state.last_error.clone(),
                queued_frames: 0,
                buffer_ms: 0.0,
                frames_captured: 0,
                frames_rendered: 0,
                underrun_frames: 0,
                dropped_frames: 0,
                playback_rate: 1.0,
                eq_enabled: false,
            },
        }
    }

    pub fn update_eq(&self, pid: u32, config: SessionEqConfig) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "流转状态锁已损坏".to_string())?;
        let Some(active) = &state.active else {
            return Ok(false);
        };
        if active.pid != pid {
            return Ok(false);
        }
        active
            .eq_tx
            .send(config)
            .map_err(|_| "实时流转线程已停止，无法更新 EQ".to_string())?;
        Ok(true)
    }

    fn refresh_finished_relay(&self) {
        let result = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .active
                .as_ref()
                .and_then(|active| active.result_rx.try_recv().ok())
        };
        let Some(result) = result else {
            return;
        };

        let active = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.active.take()
        };
        if let Some(active) = active {
            let _ = active.join.join();
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.last_error = result.err();
    }
}

struct RelayStats {
    frames_captured: u64,
    frames_rendered: u64,
    underrun_frames: u64,
    dropped_frames: u64,
}

struct RelayThreadContext {
    pid: u32,
    target_device_id: String,
    target_device_name: String,
    eq_config: SessionEqConfig,
    telemetry: Arc<Mutex<RelayTelemetry>>,
}

fn relay_process_audio(
    context: RelayThreadContext,
    stop_rx: Receiver<()>,
    eq_rx: Receiver<SessionEqConfig>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<ProcessRelayResult, String> {
    let RelayThreadContext {
        pid,
        target_device_id,
        target_device_name,
        eq_config,
        telemetry,
    } = context;
    let started_at = Instant::now();
    let _apartment =
        ComApartment::initialize().map_err(|error| format!("COM 初始化失败: {error:?}"))?;

    let initialization = (|| {
        let capture_audio_client = activate_process_audio_client(pid)?;
        let format = capture_format();
        let capture_flags = AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        unsafe {
            capture_audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                capture_flags,
                0,
                0,
                &format,
                None,
            )
        }
        .map_err(|error| format!("初始化实时捕获失败: {error:?}"))?;
        let capture_client: IAudioCaptureClient = unsafe { capture_audio_client.GetService() }
            .map_err(|error| format!("无法获取实时捕获服务: {error:?}"))?;
        let capture_event =
            OwnedEvent::create(false).map_err(|error| format!("创建捕获事件失败: {error:?}"))?;
        unsafe { capture_audio_client.SetEventHandle(capture_event.handle()) }
            .map_err(|error| format!("注册捕获事件失败: {error:?}"))?;

        let (render_audio_client, render_client, render_event, render_buffer_frames) =
            initialize_render_client(&target_device_id)?;
        let eq = ParametricEq::new(&eq_config, CAPTURE_SAMPLE_RATE, CAPTURE_CHANNELS)?;

        Ok::<_, String>((
            capture_audio_client,
            capture_client,
            capture_event,
            render_audio_client,
            render_client,
            render_event,
            render_buffer_frames,
            eq,
        ))
    })();

    let (
        capture_audio_client,
        capture_client,
        capture_event,
        render_audio_client,
        render_client,
        render_event,
        render_buffer_frames,
        eq,
    ) = match initialization {
        Ok(initialized) => initialized,
        Err(error) => {
            let _ = ready_tx.send(Err(error.clone()));
            return Err(error);
        }
    };

    if let Err(error) = unsafe { capture_audio_client.Start() } {
        let error = format!("启动实时捕获失败: {error:?}");
        let _ = ready_tx.send(Err(error.clone()));
        return Err(error);
    }
    if let Err(error) = unsafe { render_audio_client.Start() } {
        let _ = unsafe { capture_audio_client.Stop() };
        let error = format!("启动目标设备渲染失败: {error:?}");
        let _ = ready_tx.send(Err(error.clone()));
        return Err(error);
    }
    let _ = ready_tx.send(Ok(()));

    let mut queue = RelayQueue::new(eq, eq_config.enabled);
    let mut stats = RelayStats {
        frames_captured: 0,
        frames_rendered: 0,
        underrun_frames: 0,
        dropped_frames: 0,
    };
    let events = [capture_event.handle(), render_event.handle()];
    let relay_result = (|| -> Result<(), String> {
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            if let Some(config) = eq_rx.try_iter().last() {
                queue.update_eq(&config)?;
            }
            let wait_result = unsafe { WaitForMultipleObjects(&events, false, 250) };
            if wait_result == WAIT_OBJECT_0 {
                drain_capture_to_queue(&capture_client, &mut queue, &mut stats)?;
            } else if wait_result.0 == WAIT_OBJECT_0.0 + 1 {
                render_from_queue(
                    &render_audio_client,
                    &render_client,
                    render_buffer_frames,
                    &mut queue,
                    &mut stats,
                )?;
            } else if wait_result == WAIT_TIMEOUT {
                continue;
            } else if wait_result == WAIT_FAILED {
                return Err("等待实时音频事件失败".to_string());
            } else {
                return Err(format!("收到未知的音频等待结果: {}", wait_result.0));
            }
            update_telemetry(&telemetry, &stats, &queue);
        }
        Ok(())
    })();

    let _ = unsafe { capture_audio_client.Stop() };
    let _ = unsafe { render_audio_client.Stop() };
    relay_result?;
    update_telemetry(&telemetry, &stats, &queue);

    Ok(ProcessRelayResult {
        pid,
        target_device_id,
        target_device_name,
        frames_captured: stats.frames_captured,
        frames_rendered: stats.frames_rendered,
        underrun_frames: stats.underrun_frames,
        dropped_frames: stats.dropped_frames,
        duration_ms: started_at.elapsed().as_millis() as u64,
        eq_applied: eq_config.enabled,
    })
}

fn update_telemetry(
    telemetry: &Arc<Mutex<RelayTelemetry>>,
    stats: &RelayStats,
    queue: &RelayQueue,
) {
    let mut telemetry = telemetry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    telemetry.queued_frames = queue.queued_frames() as u32;
    telemetry.frames_captured = stats.frames_captured;
    telemetry.frames_rendered = stats.frames_rendered;
    telemetry.underrun_frames = stats.underrun_frames;
    telemetry.dropped_frames = stats.dropped_frames;
    telemetry.playback_rate = queue.playback_rate as f32;
    telemetry.eq_enabled = queue.eq_enabled;
}

fn initialize_render_client(
    target_device_id: &str,
) -> Result<(IAudioClient, IAudioRenderClient, OwnedEvent, u32), String> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| format!("无法创建设备枚举器: {error:?}"))?;
    let wide_id: Vec<u16> = target_device_id
        .encode_utf16()
        .chain(iter::once(0))
        .collect();
    let device = unsafe { enumerator.GetDevice(PCWSTR(wide_id.as_ptr())) }
        .map_err(|error| format!("无法打开流转目标设备: {error:?}"))?;
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| format!("无法激活流转目标设备: {error:?}"))?;
    let format = capture_format();
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
        | AUDCLNT_STREAMFLAGS_NOPERSIST;
    unsafe { audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, &format, None) }
        .map_err(|error| format!("初始化目标设备共享流失败: {error:?}"))?;
    let render_client: IAudioRenderClient = unsafe { audio_client.GetService() }
        .map_err(|error| format!("无法获取目标设备渲染服务: {error:?}"))?;
    let event =
        OwnedEvent::create(false).map_err(|error| format!("创建渲染事件失败: {error:?}"))?;
    unsafe { audio_client.SetEventHandle(event.handle()) }
        .map_err(|error| format!("注册渲染事件失败: {error:?}"))?;
    let buffer_frames = unsafe { audio_client.GetBufferSize() }
        .map_err(|error| format!("读取目标设备缓冲区大小失败: {error:?}"))?;

    if buffer_frames > 0 {
        unsafe {
            render_client.GetBuffer(buffer_frames).and_then(|_| {
                render_client.ReleaseBuffer(buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
            })
        }
        .map_err(|error| format!("预填充目标设备缓冲区失败: {error:?}"))?;
    }
    Ok((audio_client, render_client, event, buffer_frames))
}

struct RelayQueue {
    samples: VecDeque<i16>,
    float_buffer: Vec<f32>,
    next_buffer: Vec<f32>,
    eq: ParametricEq,
    next_eq: Option<ParametricEq>,
    crossfade_frames: usize,
    eq_enabled: bool,
    next_eq_enabled: bool,
    read_position: f64,
    playback_rate: f64,
    primed: bool,
}

impl RelayQueue {
    fn new(eq: ParametricEq, eq_enabled: bool) -> Self {
        Self {
            samples: VecDeque::new(),
            float_buffer: Vec::new(),
            next_buffer: Vec::new(),
            eq,
            next_eq: None,
            crossfade_frames: 0,
            eq_enabled,
            next_eq_enabled: eq_enabled,
            read_position: 0.0,
            playback_rate: 1.0,
            primed: false,
        }
    }

    fn queued_frames(&self) -> usize {
        self.samples.len() / CAPTURE_CHANNELS as usize
    }

    fn update_eq(&mut self, config: &SessionEqConfig) -> Result<(), String> {
        self.next_eq = Some(ParametricEq::new(
            config,
            CAPTURE_SAMPLE_RATE,
            CAPTURE_CHANNELS,
        )?);
        self.next_eq_enabled = config.enabled;
        self.crossfade_frames = 0;
        Ok(())
    }

    fn push_packet(&mut self, data: *const u8, frames: u32, silent: bool) -> Result<u64, String> {
        let sample_count = frames as usize * CAPTURE_CHANNELS as usize;
        self.float_buffer.clear();
        self.float_buffer.reserve(sample_count);
        if silent {
            self.float_buffer.resize(sample_count, 0.0);
        } else {
            if data.is_null() {
                return Err("Windows 返回了空的实时音频缓冲区".to_string());
            }
            let bytes = unsafe { std::slice::from_raw_parts(data, sample_count * 2) };
            self.float_buffer.extend(
                bytes
                    .chunks_exact(2)
                    .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0),
            );
        }
        if let Some(next_eq) = &mut self.next_eq {
            self.next_buffer.clear();
            self.next_buffer.extend_from_slice(&self.float_buffer);
            self.eq.process_interleaved(&mut self.float_buffer);
            next_eq.process_interleaved(&mut self.next_buffer);

            let channels = CAPTURE_CHANNELS as usize;
            for frame in 0..frames as usize {
                let alpha = ((self.crossfade_frames + frame + 1) as f32
                    / EQ_CROSSFADE_FRAMES as f32)
                    .min(1.0);
                for channel in 0..channels {
                    let index = frame * channels + channel;
                    self.float_buffer[index] =
                        self.float_buffer[index] * (1.0 - alpha) + self.next_buffer[index] * alpha;
                }
            }
            self.crossfade_frames = self.crossfade_frames.saturating_add(frames as usize);
            if self.crossfade_frames >= EQ_CROSSFADE_FRAMES {
                self.eq = self.next_eq.take().expect("下一 EQ 处理器应存在");
                self.eq_enabled = self.next_eq_enabled;
                self.crossfade_frames = 0;
            }
        } else {
            self.eq.process_interleaved(&mut self.float_buffer);
        }
        self.samples.extend(
            self.float_buffer
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16),
        );

        let max_samples = MAX_QUEUE_FRAMES * CAPTURE_CHANNELS as usize;
        let overflow_samples = self.samples.len().saturating_sub(max_samples);
        for _ in 0..overflow_samples {
            self.samples.pop_front();
        }
        Ok((overflow_samples / CAPTURE_CHANNELS as usize) as u64)
    }

    fn write_to_render_buffer(&mut self, destination: *mut u8, frames: u32) -> (u64, u64) {
        let channels = CAPTURE_CHANNELS as usize;
        let requested_frames = frames as usize;
        let requested_samples = requested_frames * channels;
        let destination =
            unsafe { std::slice::from_raw_parts_mut(destination, requested_samples * 2) };
        destination.fill(0);

        let queued_frames = self.queued_frames();
        let error = (queued_frames as f64 - PREBUFFER_FRAMES as f64) / PREBUFFER_FRAMES as f64;
        self.playback_rate = (1.0 + error * 0.0008).clamp(0.995, 1.005);

        let mut rendered_frames = 0usize;
        while rendered_frames < requested_frames {
            let source_frame = self.read_position.floor() as usize;
            if source_frame >= queued_frames {
                break;
            }
            let next_frame = (source_frame + 1).min(queued_frames - 1);
            let fraction = (self.read_position - source_frame as f64) as f32;
            for channel in 0..channels {
                let current = self.samples[source_frame * channels + channel] as f32;
                let next = self.samples[next_frame * channels + channel] as f32;
                let sample = (current + (next - current) * fraction).round() as i16;
                let bytes = sample.to_le_bytes();
                let output_index = (rendered_frames * channels + channel) * 2;
                destination[output_index] = bytes[0];
                destination[output_index + 1] = bytes[1];
            }
            rendered_frames += 1;
            self.read_position += self.playback_rate;
        }

        let consumed_frames = (self.read_position.floor() as usize).min(queued_frames);
        for _ in 0..consumed_frames * channels {
            self.samples.pop_front();
        }
        self.read_position -= consumed_frames as f64;
        if self.samples.is_empty() {
            self.read_position = 0.0;
        }

        (
            rendered_frames as u64,
            frames.saturating_sub(rendered_frames as u32) as u64,
        )
    }

    fn mark_unprimed(&mut self) {
        self.primed = false;
        self.read_position = 0.0;
    }
}

fn drain_capture_to_queue(
    capture_client: &IAudioCaptureClient,
    queue: &mut RelayQueue,
    stats: &mut RelayStats,
) -> Result<(), String> {
    loop {
        let packet_frames = unsafe { capture_client.GetNextPacketSize() }
            .map_err(|error| format!("查询实时捕获数据包失败: {error:?}"))?;
        if packet_frames == 0 {
            return Ok(());
        }

        let mut data = ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            capture_client
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .map_err(|error| format!("读取实时捕获数据包失败: {error:?}"))?;
        }
        let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
        let push_result = queue.push_packet(data, frames, silent);
        let release_result = unsafe { capture_client.ReleaseBuffer(frames) }
            .map_err(|error| format!("释放实时捕获数据包失败: {error:?}"));
        stats.frames_captured = stats.frames_captured.saturating_add(frames as u64);
        stats.dropped_frames = stats.dropped_frames.saturating_add(push_result?);
        release_result?;
    }
}

fn render_from_queue(
    audio_client: &IAudioClient,
    render_client: &IAudioRenderClient,
    buffer_frames: u32,
    queue: &mut RelayQueue,
    stats: &mut RelayStats,
) -> Result<(), String> {
    let padding = unsafe { audio_client.GetCurrentPadding() }
        .map_err(|error| format!("查询目标设备缓冲区失败: {error:?}"))?;
    let available_frames = buffer_frames.saturating_sub(padding);
    if available_frames == 0 {
        return Ok(());
    }

    if !queue.primed && queue.queued_frames() < PREBUFFER_FRAMES {
        unsafe {
            render_client.GetBuffer(available_frames).and_then(|_| {
                render_client.ReleaseBuffer(available_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
            })
        }
        .map_err(|error| format!("向目标设备写入预缓冲静音失败: {error:?}"))?;
        return Ok(());
    }
    queue.primed = true;

    let destination = unsafe { render_client.GetBuffer(available_frames) }
        .map_err(|error| format!("获取目标设备写入缓冲区失败: {error:?}"))?;
    let (rendered, underrun) = queue.write_to_render_buffer(destination, available_frames);
    unsafe { render_client.ReleaseBuffer(available_frames, 0) }
        .map_err(|error| format!("提交目标设备音频失败: {error:?}"))?;
    stats.frames_rendered = stats.frames_rendered.saturating_add(rendered);
    stats.underrun_frames = stats.underrun_frames.saturating_add(underrun);
    if underrun > 0 {
        queue.mark_unprimed();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_packet(frames: usize, sample: i16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(frames * CAPTURE_CHANNELS as usize * 2);
        for _ in 0..frames * CAPTURE_CHANNELS as usize {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    fn bypass_processor() -> ParametricEq {
        ParametricEq::new(
            &SessionEqConfig::default(),
            CAPTURE_SAMPLE_RATE,
            CAPTURE_CHANNELS,
        )
        .unwrap()
    }

    #[test]
    fn adaptive_rate_consumes_faster_when_queue_is_too_full() {
        let mut queue = RelayQueue::new(bypass_processor(), false);
        let packet = pcm_packet(PREBUFFER_FRAMES * 3, 2_000);
        queue
            .push_packet(packet.as_ptr(), (PREBUFFER_FRAMES * 3) as u32, false)
            .unwrap();
        let mut output = vec![0u8; 480 * CAPTURE_CHANNELS as usize * 2];
        let (rendered, underrun) = queue.write_to_render_buffer(output.as_mut_ptr(), 480);
        assert_eq!(rendered, 480);
        assert_eq!(underrun, 0);
        assert!(queue.playback_rate > 1.0);
    }

    #[test]
    fn adaptive_rate_consumes_slower_when_queue_is_low() {
        let mut queue = RelayQueue::new(bypass_processor(), false);
        let packet = pcm_packet(PREBUFFER_FRAMES / 2, 2_000);
        queue
            .push_packet(packet.as_ptr(), (PREBUFFER_FRAMES / 2) as u32, false)
            .unwrap();
        let mut output = vec![0u8; 480 * CAPTURE_CHANNELS as usize * 2];
        let (rendered, underrun) = queue.write_to_render_buffer(output.as_mut_ptr(), 480);
        assert_eq!(rendered, 480);
        assert_eq!(underrun, 0);
        assert!(queue.playback_rate < 1.0);
    }

    #[test]
    fn hot_eq_update_crossfades_and_becomes_active() {
        let mut queue = RelayQueue::new(bypass_processor(), false);
        let config = SessionEqConfig {
            enabled: true,
            preamp_db: 6.0,
            limiter_enabled: false,
            ..SessionEqConfig::default()
        };
        queue.update_eq(&config).unwrap();
        let packet = pcm_packet(EQ_CROSSFADE_FRAMES, 3_000);
        queue
            .push_packet(packet.as_ptr(), EQ_CROSSFADE_FRAMES as u32, false)
            .unwrap();

        assert!(queue.eq_enabled);
        assert!(queue.next_eq.is_none());
        let first = queue.samples.front().copied().unwrap();
        let last = queue.samples.back().copied().unwrap();
        assert!(first >= 3_000);
        assert!(last > first + 2_000);
    }
}
