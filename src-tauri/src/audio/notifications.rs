//! Windows 音频设备与会话事件监听。
//!
//! WASAPI 回调只向有界通道发送轻量信号；注册维护、事件合并和
//! Tauri 事件派发都在独立的 MTA 后台线程完成。

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::*,
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        },
    },
    core::{BOOL, GUID, PCWSTR, Ref, implement},
};

pub const AUDIO_HUB_EVENT_CONTEXT: GUID = GUID::from_u128(0x5635c1bd_20b7_4e34_a606_57e8b802bcaf);

pub const SESSION_CHANGED_EVENT: &str = "audio-sessions-changed";
pub const DEVICES_CHANGED_EVENT: &str = "audio-devices-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchSignal {
    SessionValuesChanged,
    SessionTopologyChanged,
    DevicesChanged,
    Shutdown,
}

fn notify(tx: &SyncSender<WatchSignal>, signal: WatchSignal) {
    let _ = tx.try_send(signal);
}

#[implement(IAudioSessionNotification)]
struct SessionNotificationCallback {
    tx: SyncSender<WatchSignal>,
}

#[allow(non_snake_case)]
impl IAudioSessionNotification_Impl for SessionNotificationCallback_Impl {
    fn OnSessionCreated(
        &self,
        _newsession: Ref<'_, IAudioSessionControl>,
    ) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::SessionTopologyChanged);
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionEventsCallback {
    tx: SyncSender<WatchSignal>,
}

impl SessionEventsCallback_Impl {
    fn notify_value_change(&self, event_context: *const GUID) {
        let originated_here =
            !event_context.is_null() && unsafe { *event_context == AUDIO_HUB_EVENT_CONTEXT };
        if !originated_here {
            notify(&self.tx, WatchSignal::SessionValuesChanged);
        }
    }
}

#[allow(non_snake_case)]
impl IAudioSessionEvents_Impl for SessionEventsCallback_Impl {
    fn OnDisplayNameChanged(
        &self,
        _newdisplayname: &PCWSTR,
        eventcontext: *const GUID,
    ) -> windows::core::Result<()> {
        self.notify_value_change(eventcontext);
        Ok(())
    }

    fn OnIconPathChanged(
        &self,
        _newiconpath: &PCWSTR,
        eventcontext: *const GUID,
    ) -> windows::core::Result<()> {
        self.notify_value_change(eventcontext);
        Ok(())
    }

    fn OnSimpleVolumeChanged(
        &self,
        _newvolume: f32,
        _newmute: BOOL,
        eventcontext: *const GUID,
    ) -> windows::core::Result<()> {
        self.notify_value_change(eventcontext);
        Ok(())
    }

    fn OnChannelVolumeChanged(
        &self,
        _channelcount: u32,
        _newchannelvolumearray: *const f32,
        _changedchannel: u32,
        eventcontext: *const GUID,
    ) -> windows::core::Result<()> {
        self.notify_value_change(eventcontext);
        Ok(())
    }

    fn OnGroupingParamChanged(
        &self,
        _newgroupingparam: *const GUID,
        eventcontext: *const GUID,
    ) -> windows::core::Result<()> {
        self.notify_value_change(eventcontext);
        Ok(())
    }

    fn OnStateChanged(&self, newstate: AudioSessionState) -> windows::core::Result<()> {
        let signal = if newstate == AudioSessionStateExpired {
            WatchSignal::SessionTopologyChanged
        } else {
            WatchSignal::SessionValuesChanged
        };
        notify(&self.tx, signal);
        Ok(())
    }

    fn OnSessionDisconnected(
        &self,
        _disconnectreason: AudioSessionDisconnectReason,
    ) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::SessionTopologyChanged);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotificationCallback {
    tx: SyncSender<WatchSignal>,
}

#[allow(non_snake_case)]
impl IMMNotificationClient_Impl for DeviceNotificationCallback_Impl {
    fn OnDeviceStateChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _dwnewstate: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::DevicesChanged);
        Ok(())
    }

    fn OnDeviceAdded(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::DevicesChanged);
        Ok(())
    }

    fn OnDeviceRemoved(&self, _pwstrdeviceid: &PCWSTR) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::DevicesChanged);
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _pwstrdefaultdeviceid: &PCWSTR,
    ) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::DevicesChanged);
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _pwstrdeviceid: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        notify(&self.tx, WatchSignal::DevicesChanged);
        Ok(())
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct AudioRegistrations {
    enumerator: IMMDeviceEnumerator,
    device_callback: IMMNotificationClient,
    session_callback: IAudioSessionNotification,
    session_events: IAudioSessionEvents,
    managers: Vec<IAudioSessionManager2>,
    session_controls: Vec<IAudioSessionControl>,
}

impl AudioRegistrations {
    fn new(tx: SyncSender<WatchSignal>) -> windows::core::Result<Self> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
        let device_callback: IMMNotificationClient =
            DeviceNotificationCallback { tx: tx.clone() }.into();
        let session_callback: IAudioSessionNotification =
            SessionNotificationCallback { tx: tx.clone() }.into();
        let session_events: IAudioSessionEvents = SessionEventsCallback { tx }.into();

        unsafe {
            enumerator.RegisterEndpointNotificationCallback(&device_callback)?;
        }

        let mut registrations = Self {
            enumerator,
            device_callback,
            session_callback,
            session_events,
            managers: Vec::new(),
            session_controls: Vec::new(),
        };
        registrations.rebuild_managers()?;
        Ok(registrations)
    }

    fn rebuild_managers(&mut self) -> windows::core::Result<()> {
        self.unregister_sessions();
        for manager in self.managers.drain(..) {
            let _ = unsafe { manager.UnregisterSessionNotification(&self.session_callback) };
        }

        let devices = unsafe {
            self.enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        }?;
        let device_count = unsafe { devices.GetCount() }?;

        for index in 0..device_count {
            let Ok(device) = (unsafe { devices.Item(index) }) else {
                continue;
            };
            let Ok(manager) =
                (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
            else {
                continue;
            };

            if unsafe { manager.RegisterSessionNotification(&self.session_callback) }.is_err() {
                continue;
            }
            self.managers.push(manager);
        }

        self.rebuild_session_events();
        Ok(())
    }

    fn rebuild_session_events(&mut self) {
        self.unregister_sessions();

        for manager in &self.managers {
            let Ok(session_enumerator) = (unsafe { manager.GetSessionEnumerator() }) else {
                continue;
            };
            // GetCount 是 Windows 开始派发 OnSessionCreated 的必要初始化步骤。
            let Ok(count) = (unsafe { session_enumerator.GetCount() }) else {
                continue;
            };
            for index in 0..count {
                let Ok(control) = (unsafe { session_enumerator.GetSession(index) }) else {
                    continue;
                };
                if unsafe { control.RegisterAudioSessionNotification(&self.session_events) }.is_ok()
                {
                    self.session_controls.push(control);
                }
            }
        }
    }

    fn unregister_sessions(&mut self) {
        for control in self.session_controls.drain(..) {
            let _ = unsafe { control.UnregisterAudioSessionNotification(&self.session_events) };
        }
    }
}

impl Drop for AudioRegistrations {
    fn drop(&mut self) {
        self.unregister_sessions();
        for manager in self.managers.drain(..) {
            let _ = unsafe { manager.UnregisterSessionNotification(&self.session_callback) };
        }
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.device_callback)
        };
    }
}

pub struct AudioNotificationWatcher {
    available: Arc<AtomicBool>,
    tx: SyncSender<WatchSignal>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AudioNotificationWatcher {
    pub fn start(app: AppHandle) -> Self {
        let available = Arc::new(AtomicBool::new(false));
        let (tx, rx) = sync_channel(32);
        let (ready_tx, ready_rx) = sync_channel(1);
        let worker_available = Arc::clone(&available);
        let worker_tx = tx.clone();
        let worker = thread::Builder::new()
            .name("audio-notification-watcher".to_string())
            .spawn(move || watcher_thread(app, worker_tx, rx, worker_available, ready_tx))
            .ok();
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));

        Self {
            available,
            tx,
            worker: Mutex::new(worker),
        }
    }

    pub fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }
}

impl Drop for AudioNotificationWatcher {
    fn drop(&mut self) {
        let _ = self.tx.try_send(WatchSignal::Shutdown);
        if let Ok(mut worker) = self.worker.lock()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn watcher_thread(
    app: AppHandle,
    tx: SyncSender<WatchSignal>,
    rx: Receiver<WatchSignal>,
    available: Arc<AtomicBool>,
    ready_tx: SyncSender<bool>,
) {
    let Ok(_apartment) = ComApartment::initialize() else {
        eprintln!("音频通知监听器初始化 COM 失败，将使用轮询降级");
        let _ = ready_tx.try_send(false);
        return;
    };
    let Ok(mut registrations) = AudioRegistrations::new(tx) else {
        eprintln!("注册 Windows 音频通知失败，将使用轮询降级");
        let _ = ready_tx.try_send(false);
        return;
    };

    available.store(true, Ordering::Release);
    let _ = ready_tx.try_send(true);
    eprintln!("Windows 音频事件监听器已启用");

    loop {
        let first_signal = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(signal) => signal,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 低频重建用于处理驱动漏报和端点内部重置。
                let _ = registrations.rebuild_managers();
                let _ = app.emit(DEVICES_CHANGED_EVENT, ());
                let _ = app.emit(SESSION_CHANGED_EVENT, ());
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if first_signal == WatchSignal::Shutdown {
            break;
        }

        let mut devices_changed = first_signal == WatchSignal::DevicesChanged;
        let mut topology_changed = first_signal == WatchSignal::SessionTopologyChanged;
        let mut session_changed = first_signal == WatchSignal::SessionValuesChanged;
        let deadline = Instant::now() + Duration::from_millis(100);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(WatchSignal::DevicesChanged) => devices_changed = true,
                Ok(WatchSignal::SessionTopologyChanged) => topology_changed = true,
                Ok(WatchSignal::SessionValuesChanged) => session_changed = true,
                Ok(WatchSignal::Shutdown) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        // 清掉合并窗口结束瞬间到达的重复信号。
        loop {
            match rx.try_recv() {
                Ok(WatchSignal::DevicesChanged) => devices_changed = true,
                Ok(WatchSignal::SessionTopologyChanged) => topology_changed = true,
                Ok(WatchSignal::SessionValuesChanged) => session_changed = true,
                Ok(WatchSignal::Shutdown) => return,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        if devices_changed {
            // 给驱动短暂时间完成端点状态切换，再重新绑定全部监听。
            thread::sleep(Duration::from_millis(100));
            let _ = registrations.rebuild_managers();
            let _ = app.emit(DEVICES_CHANGED_EVENT, ());
            let _ = app.emit(SESSION_CHANGED_EVENT, ());
        } else {
            if topology_changed {
                registrations.rebuild_session_events();
            }
            if topology_changed || session_changed {
                let _ = app.emit(SESSION_CHANGED_EVENT, ());
            }
        }
    }

    available.store(false, Ordering::Release);
    drop(registrations);
}
