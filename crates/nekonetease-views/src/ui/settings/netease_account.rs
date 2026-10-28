//! `NetEase` account form and session status binding, with the SMS and device surfaces beside
//! the password and QR ones.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nekonetease_app::cloud::{CloudService, SessionStatus};
use nekonetease_app::state::AppState;
use nekonetease_core::entities::cloud::CloudBeans;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, CloudAccount, Nav, SettingsPage};
use slint::{ComponentHandle, Weak};
use tokio::runtime::Handle;

use self::devices::DeviceController;

#[path = "netease_captcha.rs"]
mod captcha;
#[path = "netease_devices.rs"]
mod devices;
#[path = "netease_account_qr.rs"]
mod qr;

use super::netease_avatar::AvatarController;

/// Tracks bean reads by generation, so a read started before a check-in cannot land after it and
/// repaint the button as unchecked.
#[derive(Clone, Default)]
struct BeanRefresh {
    generation: Arc<AtomicU64>,
}

impl BeanRefresh {
    fn begin(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

/// Seed the account card and wire password login, the daily check-in and logout.
pub fn install(ui: &AppWindow, state: &AppState) {
    let weak = ui.as_weak();
    let avatar = AvatarController::default();
    let beans = BeanRefresh::default();
    let devices = DeviceController::default();
    paint_status(
        &weak,
        &state.cloud,
        &state.runtime,
        &avatar,
        &beans,
        &devices,
        state.cloud_playback.account_status(),
    );
    subscribe_status(weak.clone(), state, avatar, beans.clone(), devices.clone());

    let account = ui.global::<CloudAccount>();
    let qr_login = qr::QrLoginController::default();

    {
        let state = state.clone();
        let weak = weak.clone();
        account.on_login_phone(move |phone, country_code, password| {
            let state = state.clone();
            let weak = weak.clone();
            set_form_state(&weak, true, None);
            state.runtime.spawn(async move {
                if let Err(error) = state
                    .cloud_playback
                    .login_phone(phone.as_str(), country_code.as_str(), password.as_str())
                    .await
                {
                    log::warn!("netease account phone login: {}", describe(&error));
                    report_failure(&weak, &error);
                } else {
                    set_form_state(&weak, false, None);
                }
            });
        });
    }

    {
        let state = state.clone();
        let weak = weak.clone();
        account.on_login_email(move |email, password| {
            let state = state.clone();
            let weak = weak.clone();
            set_form_state(&weak, true, None);
            state.runtime.spawn(async move {
                if let Err(error) =
                    state.cloud_playback.login_email(email.as_str(), password.as_str()).await
                {
                    log::warn!("netease account email login: {}", describe(&error));
                    report_failure(&weak, &error);
                } else {
                    set_form_state(&weak, false, None);
                }
            });
        });
    }

    {
        let state = state.clone();
        let weak = weak.clone();
        let qr_login = qr_login.clone();
        account.on_start_qr_login(move || {
            qr_login.start(weak.clone(), state.clone(), false);
        });
    }

    {
        let weak = weak.clone();
        let qr_login = qr_login.clone();
        account.on_cancel_qr_login(move || qr_login.cancel(&weak));
    }

    {
        let weak = weak.clone();
        let qr_login = qr_login.clone();
        let state = state.clone();
        account.on_qr_page_active_changed(move |active| {
            if active {
                qr_login.start(weak.clone(), state.clone(), true);
            } else {
                qr_login.cancel(&weak);
            }
        });
    }

    captcha::install(&account, state, &weak);
    devices::install(&account, state, &weak, &devices);

    {
        let state = state.clone();
        let weak = weak.clone();
        let beans = beans.clone();
        account.on_check_in(move || check_in(&weak, &state, &beans));
    }

    {
        let state = state.clone();
        let weak = weak.clone();
        let qr_login = qr_login.clone();
        account.on_logout(move || {
            let state = state.clone();
            let weak = weak.clone();
            qr_login.cancel(&weak);
            set_form_state(&weak, true, None);
            state.runtime.spawn(async move {
                if let Err(error) = state.cloud_playback.logout().await {
                    log::warn!("netease account logout: {}", describe(&error));
                    report_failure(&weak, &error);
                } else {
                    set_form_state(&weak, false, None);
                }
            });
        });
    }

    wire_social_entries(&account, state, &weak);
}

/// The account card's three ways into the social surface. The page comes in as a mode rather than
/// an id because the labels are Slint literals, and the own account id is read here rather than
/// carried in a property.
fn wire_social_entries(account: &CloudAccount, state: &AppState, weak: &Weak<AppWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    account.on_open_social(move |page| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(social) = crate::ui::social::social() else { return };
        match page {
            0 => {
                if let Some(account) = state.cloud.status().account {
                    social.open_profile(&ui, account.id);
                }
            }
            1 => social.open_messages(&ui),
            2 => social.open_notifications(&ui),
            _ => {}
        }
    });
}

/// Start the first QR code after persisted tabs have been hydrated. Slint initializes change
/// trackers silently, so a launch already on Services may not emit the page-active callback.
pub fn start_initial_qr_login_if_active(ui: &AppWindow) {
    if qr_page_is_active(ui) {
        ui.global::<CloudAccount>().invoke_qr_page_active_changed(true);
    }
}

/// Start QR sign-in when the optional welcome guide is opened, even if the underlying Settings tab
/// is not Services. The automatic path waits for session restoration before doing any login work.
pub fn start_onboarding_qr_login_if_needed(ui: &AppWindow) {
    let account = ui.global::<CloudAccount>();
    if !qr_page_is_active(ui) && !account.get_logged_in() && account.get_qr_status() == 0 {
        account.invoke_qr_page_active_changed(true);
    }
}

/// Keep QR polling alive after the guide closes only when the underlying Services page is active.
pub fn stop_onboarding_qr_login_if_page_inactive(ui: &AppWindow) {
    if !qr_page_is_active(ui) {
        ui.global::<CloudAccount>().invoke_qr_page_active_changed(false);
    }
}

fn qr_page_is_active(ui: &AppWindow) -> bool {
    const NAV_SETTINGS: i32 = 9;

    let nav = ui.global::<Nav>();
    let settings = ui.global::<SettingsPage>();
    nav.get_selected_index() == NAV_SETTINGS
        && !nav.get_now_playing_open()
        && (settings.get_tab_idx() == settings.get_tab_services()
            || !settings.get_search_query().is_empty())
}

fn report_failure(weak: &Weak<AppWindow>, error: &nekonetease_core::error::AppError) {
    let detail = describe(error);
    toast::notify(ToastKind::OperationFailed, detail.clone());
    set_form_state(weak, false, Some(detail));
}

fn subscribe_status(
    weak: Weak<AppWindow>,
    state: &AppState,
    avatar: AvatarController,
    beans: BeanRefresh,
    devices: DeviceController,
) {
    let mut status = state.cloud_playback.subscribe_account();
    let cloud = state.cloud.clone();
    let runtime = state.runtime.clone();
    state.runtime.spawn(async move {
        while status.changed().await.is_ok() {
            paint_status(
                &weak,
                &cloud,
                &runtime,
                &avatar,
                &beans,
                &devices,
                status.borrow().clone(),
            );
        }
    });
}

fn paint_status(
    weak: &Weak<AppWindow>,
    cloud: &CloudService,
    runtime: &Handle,
    avatar: &AvatarController,
    beans: &BeanRefresh,
    devices: &DeviceController,
    status: SessionStatus,
) {
    let avatar_url = status.account.as_ref().and_then(|account| account.avatar_url.clone());
    avatar.apply(weak, cloud, runtime, avatar_url);
    let logged_in = status.account.is_some();
    if logged_in {
        refresh_beans(weak, cloud, runtime, beans);
    } else {
        beans.invalidate();
    }
    devices::sync_with_status(weak, cloud, runtime, devices, logged_in);
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let account = ui.global::<CloudAccount>();
        let (account_name, account_level, account_vip) = status.account.as_ref().map_or_else(
            || (String::new(), 0, false),
            |account| (account.nickname.clone(), account.level, account.vip_type != 0),
        );
        account.set_logged_in(logged_in);
        account.set_account_name(account_name.into());
        account.set_account_level(account_level);
        account.set_account_vip(account_vip);
        account.set_remember_available(status.remember_available);
        if !logged_in {
            account.set_bean_balance(0);
            account.set_beans_loaded(false);
            account.set_signed_today(false);
            account.set_beans_busy(false);
        }
        // A sign-in transition briefly publishes a signed-out status before the request resolves.
        // Preserve the request error set by the form during that interval.
        if logged_in || status.notice.is_some() {
            account.set_notice(status.notice.unwrap_or_default().into());
        }
    });
}

fn refresh_beans(
    weak: &Weak<AppWindow>,
    cloud: &CloudService,
    runtime: &Handle,
    refresh: &BeanRefresh,
) {
    let generation = refresh.begin();
    let refresh = refresh.clone();
    let cloud = cloud.clone();
    let weak = weak.clone();
    runtime.spawn(async move {
        let result = cloud.cloud_beans().await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !refresh.is_current(generation) {
                return;
            }
            match result {
                Ok(beans) => apply_beans(&ui, beans),
                Err(error) => log::warn!("netease cloud beans: {}", describe(&error)),
            }
        });
    });
}

fn apply_beans(ui: &AppWindow, beans: CloudBeans) {
    let account = ui.global::<CloudAccount>();
    account.set_bean_balance(i32::try_from(beans.balance).unwrap_or(i32::MAX));
    account.set_signed_today(beans.signed_today);
    account.set_beans_loaded(true);
}

fn check_in(weak: &Weak<AppWindow>, state: &AppState, refresh: &BeanRefresh) {
    set_beans_busy(weak, true);
    let weak = weak.clone();
    let cloud = state.cloud.clone();
    let runtime = state.runtime.clone();
    let refresh = refresh.clone();
    runtime.clone().spawn(async move {
        match cloud.check_in().await {
            Ok(claimed) => {
                if claimed {
                    toast::notify(ToastKind::CheckInSucceeded, "");
                }
                let _ = weak.upgrade_in_event_loop(|ui| {
                    let account = ui.global::<CloudAccount>();
                    if !account.get_logged_in() {
                        return;
                    }
                    // `sign: false` is the repeat claim, which still means the day is done.
                    account.set_signed_today(true);
                    account.set_beans_busy(false);
                });
                refresh_beans(&weak, &cloud, &runtime, &refresh);
            }
            Err(error) => {
                log::warn!("netease check-in: {}", describe(&error));
                toast::notify(ToastKind::OperationFailed, describe(&error));
                set_beans_busy(&weak, false);
            }
        }
    });
}

fn set_beans_busy(weak: &Weak<AppWindow>, busy: bool) {
    let _ = weak.clone().upgrade_in_event_loop(move |ui| {
        ui.global::<CloudAccount>().set_beans_busy(busy);
    });
}

pub(super) fn set_form_state(weak: &Weak<AppWindow>, busy: bool, notice: Option<String>) {
    let notice = notice.unwrap_or_default();
    let _ = weak.clone().upgrade_in_event_loop(move |ui| {
        let account = ui.global::<CloudAccount>();
        account.set_busy(busy);
        account.set_notice(notice.into());
    });
}

#[cfg(test)]
#[path = "tests/netease_account_tests.rs"]
mod tests;
