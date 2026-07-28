use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    mem::{ManuallyDrop, size_of},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use windows::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{
            CloseHandle, HANDLE, RPC_E_CHANGED_MODE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
            ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVE_FORMAT_PCM, WAVEFORMATEX,
        },
        System::{
            Com::{
                BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize, IAgileObject,
                IAgileObject_Impl,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            SystemInformation::OSVERSIONINFOW,
            Threading::{CreateEventW, SetEvent, WaitForSingleObject},
            Variant::VT_BLOB,
        },
    },
    core::{HRESULT, IUnknown, Interface, Ref, Result as WindowsResult, implement, w},
};

const MIN_PROCESS_LOOPBACK_BUILD: u32 = 20_348;
pub(super) const CAPTURE_SAMPLE_RATE: u32 = 48_000;
pub(super) const CAPTURE_CHANNELS: u16 = 2;
pub(super) const CAPTURE_BITS_PER_SAMPLE: u16 = 16;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessLoopbackSupport {
    pub supported: bool,
    pub windows_build: u32,
    pub minimum_build: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessCaptureStatus {
    pub supported: bool,
    pub windows_build: u32,
    pub active: bool,
    pub pid: Option<u32>,
    pub output_path: Option<String>,
    pub elapsed_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessCaptureResult {
    pub pid: u32,
    pub output_path: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_written: u64,
    pub bytes_written: u64,
    pub duration_ms: u64,
}

struct ActiveCapture {
    pid: u32,
    output_path: PathBuf,
    started_at: Instant,
    stop_tx: Sender<()>,
    result_rx: Receiver<Result<ProcessCaptureResult, String>>,
    join: JoinHandle<()>,
}

#[derive(Default)]
struct ManagerState {
    active: Option<ActiveCapture>,
    last_output_path: Option<PathBuf>,
    last_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct ProcessCaptureManager {
    state: Arc<Mutex<ManagerState>>,
}

impl ProcessCaptureManager {
    pub fn support() -> ProcessLoopbackSupport {
        let windows_build = windows_build_number();
        ProcessLoopbackSupport {
            supported: windows_build >= MIN_PROCESS_LOOPBACK_BUILD,
            windows_build,
            minimum_build: MIN_PROCESS_LOOPBACK_BUILD,
        }
    }

    pub fn start(&self, pid: u32, output_dir: &Path) -> Result<ProcessCaptureStatus, String> {
        if pid == 0 {
            return Err("系统声音不支持按进程捕获，请选择一个应用会话".to_string());
        }

        let support = Self::support();
        if !support.supported {
            return Err(format!(
                "当前 Windows Build {} 不支持进程音频捕获，最低需要 Build {}",
                support.windows_build, support.minimum_build
            ));
        }

        self.refresh_finished_capture();
        {
            let state = self
                .state
                .lock()
                .map_err(|_| "捕获状态锁已损坏".to_string())?;
            if let Some(active) = &state.active {
                return Err(format!("PID {} 正在录制，请先停止当前录制", active.pid));
            }
        }

        fs::create_dir_all(output_dir).map_err(|error| format!("无法创建录制目录: {error}"))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let output_path = output_dir.join(format!("process-{pid}-{timestamp}.wav"));

        let (stop_tx, stop_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let thread_output_path = output_path.clone();
        let join = thread::Builder::new()
            .name(format!("audio-hub-capture-{pid}"))
            .spawn(move || {
                let result = capture_process_audio(pid, &thread_output_path, stop_rx, ready_tx);
                let _ = result_tx.send(result);
            })
            .map_err(|error| format!("无法启动捕获线程: {error}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "捕获状态锁已损坏".to_string())?;
                state.last_error = None;
                state.active = Some(ActiveCapture {
                    pid,
                    output_path,
                    started_at: Instant::now(),
                    stop_tx,
                    result_rx,
                    join,
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
                Err("初始化进程音频捕获超时".to_string())
            }
        }
    }

    pub fn stop(&self) -> Result<ProcessCaptureResult, String> {
        self.refresh_finished_capture();
        let active = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "捕获状态锁已损坏".to_string())?;
            state
                .active
                .take()
                .ok_or_else(|| "当前没有正在进行的录制".to_string())?
        };

        let _ = active.stop_tx.send(());
        let _ = active.join.join();
        let result = active
            .result_rx
            .recv()
            .map_err(|_| "捕获线程未返回结果".to_string())?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| "捕获状态锁已损坏".to_string())?;
        match &result {
            Ok(capture) => {
                state.last_output_path = Some(PathBuf::from(&capture.output_path));
                state.last_error = None;
            }
            Err(error) => {
                state.last_error = Some(error.clone());
            }
        }
        result
    }

    pub fn status(&self) -> ProcessCaptureStatus {
        self.refresh_finished_capture();
        let support = Self::support();
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (active, pid, output_path, elapsed_ms) = match &state.active {
            Some(active) => (
                true,
                Some(active.pid),
                Some(active.output_path.to_string_lossy().into_owned()),
                active.started_at.elapsed().as_millis() as u64,
            ),
            None => (
                false,
                None,
                state
                    .last_output_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                0,
            ),
        };

        ProcessCaptureStatus {
            supported: support.supported,
            windows_build: support.windows_build,
            active,
            pid,
            output_path,
            elapsed_ms,
            last_error: state.last_error.clone(),
        }
    }

    fn refresh_finished_capture(&self) {
        let finished_result = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.active.as_ref().and_then(|active| {
                active
                    .result_rx
                    .try_recv()
                    .ok()
                    .map(|result| (active.pid, result))
            })
        };

        let Some((_pid, result)) = finished_result else {
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
        match result {
            Ok(capture) => {
                state.last_output_path = Some(PathBuf::from(capture.output_path));
                state.last_error = None;
            }
            Err(error) => state.last_error = Some(error),
        }
    }
}

pub(super) struct ComApartment(bool);

impl ComApartment {
    pub(super) fn initialize() -> WindowsResult<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self(false));
        }
        result.ok()?;
        Ok(Self(true))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct ActivationHandler {
    done: HANDLE,
}

impl IAgileObject_Impl for ActivationHandler_Impl {}

#[allow(non_snake_case)]
impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> WindowsResult<()> {
        unsafe { SetEvent(self.done) }
    }
}

pub(super) fn activate_process_audio_client(pid: u32) -> Result<IAudioClient, String> {
    let mut activation_params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // PROPVARIANT::drop 会调用 PropVariantClear。这里的 BLOB 指向栈上的
    // activation_params，不能让 PropVariantClear 尝试释放该指针。
    let prop_variant = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (&mut activation_params as *mut AUDIOCLIENT_ACTIVATION_PARAMS)
                            .cast::<u8>(),
                    },
                },
            }),
        },
    });

    let done_event =
        OwnedEvent::create(true).map_err(|error| format!("创建音频接口激活事件失败: {error:?}"))?;
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        done: done_event.handle(),
    }
    .into();

    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop_variant),
            &handler,
        )
    }
    .map_err(|error| format!("无法启动进程音频接口: {error:?}"))?;

    let wait_result = unsafe { WaitForSingleObject(done_event.handle(), 10_000) };
    if wait_result == WAIT_TIMEOUT {
        return Err("等待 Windows 音频接口激活超时".to_string());
    }
    if wait_result != WAIT_OBJECT_0 {
        return Err(format!("等待 Windows 音频接口失败: {}", wait_result.0));
    }

    // 在发起激活的捕获线程中取回 COM 接口，不把 IAudioClient 跨线程传递。
    let mut activation_result = HRESULT(0);
    let mut activated: Option<IUnknown> = None;
    unsafe {
        operation
            .GetActivateResult(&mut activation_result, &mut activated)
            .map_err(|error| format!("读取音频接口激活结果失败: {error:?}"))?;
    }
    activation_result
        .ok()
        .map_err(|error| format!("进程音频接口激活失败: {error:?}"))?;
    activated
        .ok_or_else(|| "Windows 返回了空的音频接口".to_string())?
        .cast::<IAudioClient>()
        .map_err(|error| format!("无法转换为 IAudioClient: {error:?}"))
}

fn capture_process_audio(
    pid: u32,
    output_path: &Path,
    stop_rx: Receiver<()>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<ProcessCaptureResult, String> {
    let started_at = Instant::now();
    let _apartment =
        ComApartment::initialize().map_err(|error| format!("COM 初始化失败: {error:?}"))?;
    let init_result = initialize_capture(pid, output_path);
    let (audio_client, capture_client, event, mut writer) = match init_result {
        Ok(initialized) => initialized,
        Err(error) => {
            let _ = ready_tx.send(Err(error.clone()));
            return Err(error);
        }
    };

    if let Err(error) = unsafe { audio_client.Start() } {
        let error = format!("启动音频捕获失败: {error:?}");
        let _ = ready_tx.send(Err(error.clone()));
        return Err(error);
    }
    let _ = ready_tx.send(Ok(()));

    let capture_result = (|| -> Result<(), String> {
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }

            let wait_result = unsafe { WaitForSingleObject(event.handle(), 250) };
            if wait_result == WAIT_OBJECT_0 {
                drain_capture_packets(&capture_client, &mut writer)?;
            } else if wait_result == WAIT_TIMEOUT {
                continue;
            } else if wait_result == WAIT_FAILED {
                return Err("等待音频采样事件失败".to_string());
            } else {
                return Err(format!("收到未知的音频等待结果: {}", wait_result.0));
            }
        }
        drain_capture_packets(&capture_client, &mut writer)?;
        Ok(())
    })();

    let _ = unsafe { audio_client.Stop() };
    capture_result?;
    let (frames_written, bytes_written) = writer.finish()?;

    Ok(ProcessCaptureResult {
        pid,
        output_path: output_path.to_string_lossy().into_owned(),
        sample_rate: CAPTURE_SAMPLE_RATE,
        channels: CAPTURE_CHANNELS,
        frames_written,
        bytes_written,
        duration_ms: started_at.elapsed().as_millis() as u64,
    })
}

fn initialize_capture(
    pid: u32,
    output_path: &Path,
) -> Result<(IAudioClient, IAudioCaptureClient, OwnedEvent, WaveWriter), String> {
    let audio_client = activate_process_audio_client(pid)?;
    let format = capture_format();
    let stream_flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

    unsafe { audio_client.Initialize(AUDCLNT_SHAREMODE_SHARED, stream_flags, 0, 0, &format, None) }
        .map_err(|error| format!("初始化共享模式捕获失败: {error:?}"))?;

    let capture_client: IAudioCaptureClient = unsafe { audio_client.GetService() }
        .map_err(|error| format!("无法获取音频捕获服务: {error:?}"))?;
    let event =
        OwnedEvent::create(false).map_err(|error| format!("创建采样事件失败: {error:?}"))?;
    unsafe { audio_client.SetEventHandle(event.handle()) }
        .map_err(|error| format!("注册采样事件失败: {error:?}"))?;
    let writer = WaveWriter::create(output_path, format)?;
    Ok((audio_client, capture_client, event, writer))
}

fn drain_capture_packets(
    capture_client: &IAudioCaptureClient,
    writer: &mut WaveWriter,
) -> Result<(), String> {
    loop {
        let packet_frames = unsafe { capture_client.GetNextPacketSize() }
            .map_err(|error| format!("查询音频数据包失败: {error:?}"))?;
        if packet_frames == 0 {
            return Ok(());
        }

        let mut data = ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            capture_client
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .map_err(|error| format!("读取音频数据包失败: {error:?}"))?;
        }

        let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
        let write_result = writer.write_frames(data, frames, silent);
        let release_result = unsafe { capture_client.ReleaseBuffer(frames) }
            .map_err(|error| format!("释放音频数据包失败: {error:?}"));
        write_result?;
        release_result?;
    }
}

pub(super) fn capture_format() -> WAVEFORMATEX {
    let block_align = CAPTURE_CHANNELS * CAPTURE_BITS_PER_SAMPLE / 8;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: CAPTURE_CHANNELS,
        nSamplesPerSec: CAPTURE_SAMPLE_RATE,
        nAvgBytesPerSec: CAPTURE_SAMPLE_RATE * block_align as u32,
        nBlockAlign: block_align,
        wBitsPerSample: CAPTURE_BITS_PER_SAMPLE,
        cbSize: 0,
    }
}

pub(super) struct OwnedEvent(HANDLE);

impl OwnedEvent {
    pub(super) fn create(manual_reset: bool) -> WindowsResult<Self> {
        unsafe { CreateEventW(None, manual_reset, false, w!("")) }.map(Self)
    }

    pub(super) fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct WaveWriter {
    file: File,
    format: WAVEFORMATEX,
    data_bytes: u64,
    frames: u64,
}

impl WaveWriter {
    fn create(path: &Path, format: WAVEFORMATEX) -> Result<Self, String> {
        let mut file = File::create(path).map_err(|error| format!("无法创建 WAV 文件: {error}"))?;
        write_wav_header(&mut file, format, 0)?;
        Ok(Self {
            file,
            format,
            data_bytes: 0,
            frames: 0,
        })
    }

    fn write_frames(&mut self, data: *const u8, frames: u32, silent: bool) -> Result<(), String> {
        let byte_count = frames as usize * self.format.nBlockAlign as usize;
        if silent {
            const ZEROES: [u8; 8192] = [0; 8192];
            let mut remaining = byte_count;
            while remaining > 0 {
                let chunk_size = remaining.min(ZEROES.len());
                self.file
                    .write_all(&ZEROES[..chunk_size])
                    .map_err(|error| format!("写入静音音频失败: {error}"))?;
                remaining -= chunk_size;
            }
        } else {
            if data.is_null() {
                return Err("Windows 返回了空的音频数据缓冲区".to_string());
            }
            let bytes = unsafe { std::slice::from_raw_parts(data, byte_count) };
            self.file
                .write_all(bytes)
                .map_err(|error| format!("写入音频数据失败: {error}"))?;
        }

        self.data_bytes = self.data_bytes.saturating_add(byte_count as u64);
        self.frames = self.frames.saturating_add(frames as u64);
        Ok(())
    }

    fn finish(mut self) -> Result<(u64, u64), String> {
        if self.data_bytes > u32::MAX as u64 {
            return Err("WAV 文件超过 4GB 上限".to_string());
        }
        let data_size = self.data_bytes as u32;
        self.file
            .seek(SeekFrom::Start(4))
            .and_then(|_| self.file.write_all(&(36u32 + data_size).to_le_bytes()))
            .and_then(|_| self.file.seek(SeekFrom::Start(40)))
            .and_then(|_| self.file.write_all(&data_size.to_le_bytes()))
            .and_then(|_| self.file.flush())
            .map_err(|error| format!("完成 WAV 文件失败: {error}"))?;
        Ok((self.frames, self.data_bytes))
    }
}

fn write_wav_header(file: &mut File, format: WAVEFORMATEX, data_size: u32) -> Result<(), String> {
    file.write_all(b"RIFF")
        .and_then(|_| file.write_all(&(36u32 + data_size).to_le_bytes()))
        .and_then(|_| file.write_all(b"WAVE"))
        .and_then(|_| file.write_all(b"fmt "))
        .and_then(|_| file.write_all(&16u32.to_le_bytes()))
        .and_then(|_| file.write_all(&format.wFormatTag.to_le_bytes()))
        .and_then(|_| file.write_all(&format.nChannels.to_le_bytes()))
        .and_then(|_| file.write_all(&format.nSamplesPerSec.to_le_bytes()))
        .and_then(|_| file.write_all(&format.nAvgBytesPerSec.to_le_bytes()))
        .and_then(|_| file.write_all(&format.nBlockAlign.to_le_bytes()))
        .and_then(|_| file.write_all(&format.wBitsPerSample.to_le_bytes()))
        .and_then(|_| file.write_all(b"data"))
        .and_then(|_| file.write_all(&data_size.to_le_bytes()))
        .map_err(|error| format!("写入 WAV 文件头失败: {error}"))
}

fn windows_build_number() -> u32 {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    if unsafe { RtlGetVersion(&mut version) }.0 >= 0 {
        version.dwBuildNumber
    } else {
        0
    }
}
