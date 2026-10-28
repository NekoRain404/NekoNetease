//! The signed-in device list: loading it, publishing parallel rows, and revoking one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use nekonetease_app::cloud::CloudService;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, CloudAccount};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;

#[derive(Clone, Default)]
pub(super) struct DeviceController {
    generation: Arc<AtomicU64>,
    loaded: Arc<AtomicBool>,
    refreshing: Arc<AtomicBool>,
}

pub(super) fn install(
    account: &CloudAccount<'_>,
    state: &AppState,
    weak: &Weak<AppWindow>,
    controller: &DeviceController,
) {
    {
        let state = state.clone();
        let weak = weak.clone();
        let controller = controller.clone();
        account.on_refresh_devices(move || {
            refresh(&weak, &state.cloud, &state.runtime, &controller);
        });
    }

    {
        let state = state.clone();
        let weak = weak.clone();
        let controller = controller.clone();
        account.on_revoke_device(move |key| {
            let key = key.to_string();
            let state = state.clone();
            let weak = weak.clone();
            let controller = controller.clone();
            set_busy(&weak, true, None);
            let runtime = state.runtime.clone();
            runtime.spawn(async move {
                match state.cloud.revoke_device(&key).await {
                    Ok(()) => refresh(&weak, &state.cloud, &state.runtime, &controller),
                    Err(error) => {
                        log::warn!("netease device revoke: {}", describe(&error));
                        toast::notify(ToastKind::OperationFailed, describe(&error));
                        set_busy(&weak, false, Some(describe(&error)));
                    }
                }
            });
        });
    }
}

/// Loads the devices once after the account signs in, and hands them back on sign-out.
pub(super) fn sync_with_status(
    weak: &Weak<AppWindow>,
    cloud: &CloudService,
    runtime: &Handle,
    controller: &DeviceController,
    logged_in: bool,
) {
    if logged_in {
        if !controller.loaded.load(Ordering::Acquire) {
            refresh(weak, cloud, runtime, controller);
        }
    } else {
        clear(weak, controller);
    }
}

fn refresh(
    weak: &Weak<AppWindow>,
    cloud: &CloudService,
    runtime: &Handle,
    controller: &DeviceController,
) {
    let Some(generation) = controller.begin() else {
        return;
    };
    set_busy(weak, true, None);

    let weak = weak.clone();
    let cloud = cloud.clone();
    let controller = controller.clone();
    runtime.spawn(async move {
        let result = cloud.devices().await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !controller.is_current(generation) {
                return;
            }
            let account = ui.global::<CloudAccount>();
            account.set_devices_busy(false);
            match result {
                Ok(devices) => {
                    controller.succeed();
                    account.set_device_notice(SharedString::default());
                    let now_ms = Utc::now().timestamp_millis();
                    if devices.is_empty() {
                        account.set_device_keys(empty_strings());
                        account.set_device_labels(empty_strings());
                        account.set_device_details(empty_strings());
                        account
                            .set_device_current(ModelRc::new(VecModel::from(Vec::<bool>::new())));
                    } else {
                        let mut keys: Vec<SharedString> = Vec::with_capacity(devices.len());
                        let mut labels: Vec<SharedString> = Vec::with_capacity(devices.len());
                        let mut details: Vec<SharedString> = Vec::with_capacity(devices.len());
                        let mut current: Vec<bool> = Vec::with_capacity(devices.len());
                        for device in &devices {
                            keys.push(device.key.as_str().into());
                            labels.push(device_label(&device.name, &device.os).into());
                            details.push(
                                device_details(
                                    &device.os,
                                    &device.place,
                                    device.last_active_ms,
                                    now_ms,
                                )
                                .into(),
                            );
                            current.push(device.current);
                        }
                        // Keys last: the row count follows this model, so every other array is
                        // already sized for the indexes the section reads.
                        account.set_device_labels(ModelRc::new(VecModel::from(labels)));
                        account.set_device_details(ModelRc::new(VecModel::from(details)));
                        account.set_device_current(ModelRc::new(VecModel::from(current)));
                        account.set_device_keys(ModelRc::new(VecModel::from(keys)));
                    }
                    account.set_devices_loaded(true);
                }
                Err(error) => {
                    controller.fail(generation);
                    let detail = describe(&error);
                    log::warn!("netease device list: {detail}");
                    account.set_device_notice(detail.clone().into());
                    toast::notify(ToastKind::OperationFailed, detail);
                }
            }
        });
    });
}

fn empty_strings() -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(Vec::<SharedString>::new()))
}

fn clear(weak: &Weak<AppWindow>, controller: &DeviceController) {
    controller.invalidate();
    let _ = weak.clone().upgrade_in_event_loop(|ui| {
        let account = ui.global::<CloudAccount>();
        account.set_device_keys(empty_strings());
        account.set_device_labels(empty_strings());
        account.set_device_details(empty_strings());
        account.set_device_current(ModelRc::new(VecModel::from(Vec::<bool>::new())));
        account.set_devices_loaded(false);
        account.set_devices_busy(false);
        account.set_device_notice(SharedString::default());
    });
}

fn set_busy(weak: &Weak<AppWindow>, busy: bool, notice: Option<String>) {
    let notice = notice.unwrap_or_default();
    let _ = weak.clone().upgrade_in_event_loop(move |ui| {
        let account = ui.global::<CloudAccount>();
        account.set_devices_busy(busy);
        account.set_device_notice(notice.into());
    });
}

pub(super) fn device_label(name: &str, os: &str) -> String {
    if !name.trim().is_empty() {
        name.trim().to_owned()
    } else if !os.trim().is_empty() {
        os.trim().to_owned()
    } else {
        "未知设备".to_owned()
    }
}

pub(super) fn device_details(os: &str, place: &str, last_active_ms: i64, now_ms: i64) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(3);
    if !os.trim().is_empty() {
        parts.push(os.trim());
    }
    if !place.trim().is_empty() {
        parts.push(place.trim());
    }
    let active = last_active_phrase(now_ms, last_active_ms);
    if !active.is_empty() {
        parts.push(&active);
    }
    parts.join(" · ")
}

/// How long ago the device last reported in. An absent timestamp draws nothing rather than a
/// misleading "just now".
pub(super) fn last_active_phrase(now_ms: i64, last_active_ms: i64) -> String {
    if last_active_ms <= 0 {
        return String::new();
    }
    let elapsed_ms = now_ms.saturating_sub(last_active_ms).max(0);
    let minutes = elapsed_ms / 60_000;
    if minutes < 1 {
        return "刚刚".to_owned();
    }
    if minutes < 60 {
        return format!("{minutes} 分钟前");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} 小时前");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days} 天前");
    }
    DateTime::<Utc>::from_timestamp_millis(last_active_ms)
        .map(|stamp| stamp.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

impl DeviceController {
    fn begin(&self) -> Option<u64> {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1))
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn succeed(&self) {
        self.loaded.store(true, Ordering::Release);
        self.refreshing.store(false, Ordering::Release);
    }

    fn fail(&self, generation: u64) {
        if self.is_current(generation) {
            self.loaded.store(false, Ordering::Release);
            self.refreshing.store(false, Ordering::Release);
        }
    }

    fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.loaded.store(false, Ordering::Release);
        self.refreshing.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "tests/netease_devices_tests.rs"]
mod tests;
