//! SMS login: requesting a code, redeeming it, and the resend countdown.
//!
//! The countdown is Rust's rather than a Slint `Timer` so it survives the form being
//! unmounted by a login-mode switch.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_core::utils::toast::{self, ToastKind};
use nekonetease_ui::{AppWindow, CloudAccount};
use slint::{ComponentHandle, Weak};
use tokio::runtime::Handle;

use super::set_form_state;

const RESEND_DELAY_SECONDS: u64 = 60;

#[derive(Clone, Default)]
struct Cooldown {
    generation: Arc<AtomicU64>,
}

pub(super) fn install(account: &CloudAccount<'_>, state: &AppState, weak: &Weak<AppWindow>) {
    let cooldown = Cooldown::default();

    {
        let state = state.clone();
        let weak = weak.clone();
        let cooldown = cooldown.clone();
        account.on_send_login_captcha(move |phone, country_code| {
            let state = state.clone();
            let weak = weak.clone();
            let cooldown = cooldown.clone();
            set_form_state(&weak, false, None);
            set_sending(&weak, true);
            let runtime = state.runtime.clone();
            runtime.spawn(async move {
                let result =
                    state.cloud.send_login_captcha(phone.as_str(), country_code.as_str()).await;
                match result {
                    Ok(()) => {
                        set_sending(&weak, false);
                        start_cooldown(&weak, &state.runtime, &cooldown);
                    }
                    Err(error) => {
                        log::warn!("netease sms code: {}", describe(&error));
                        set_sending(&weak, false);
                        report_failure(&weak, &error);
                    }
                }
            });
        });
    }

    {
        let state = state.clone();
        let weak = weak.clone();
        account.on_login_phone_captcha(move |phone, country_code, captcha| {
            let state = state.clone();
            let weak = weak.clone();
            set_form_state(&weak, true, None);
            let runtime = state.runtime.clone();
            runtime.spawn(async move {
                let result = state
                    .cloud
                    .login_phone_captcha(phone.as_str(), country_code.as_str(), captcha.as_str())
                    .await;
                match result {
                    Ok(_) => set_form_state(&weak, false, None),
                    Err(error) => {
                        log::warn!("netease sms login: {}", describe(&error));
                        report_failure(&weak, &error);
                    }
                }
            });
        });
    }
}

fn report_failure(weak: &Weak<AppWindow>, error: &nekonetease_core::error::AppError) {
    let detail = describe(error);
    toast::notify(ToastKind::OperationFailed, detail.clone());
    set_form_state(weak, false, Some(detail));
}

fn set_sending(weak: &Weak<AppWindow>, sending: bool) {
    let _ = weak.clone().upgrade_in_event_loop(move |ui| {
        ui.global::<CloudAccount>().set_captcha_sending(sending);
    });
}

fn start_cooldown(weak: &Weak<AppWindow>, runtime: &Handle, cooldown: &Cooldown) {
    let generation = cooldown.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let weak = weak.clone();
    let current = cooldown.generation.clone();
    runtime.spawn(async move {
        let started = Instant::now();
        loop {
            let remaining = cooldown_remaining(started.elapsed());
            let done = remaining == 0;
            let weak_tick = weak.clone();
            let current_tick = current.clone();
            let _ = weak_tick.upgrade_in_event_loop(move |ui| {
                if current_tick.load(Ordering::Acquire) == generation {
                    ui.global::<CloudAccount>().set_captcha_cooldown(remaining);
                }
            });
            if done {
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// Seconds still to wait, from the time the code request succeeded. A partial second reads as
/// the second it is in, so the button only frees on the tick that reaches the delay.
pub(super) fn cooldown_remaining(elapsed: Duration) -> i32 {
    let remaining = RESEND_DELAY_SECONDS.saturating_sub(elapsed.as_secs());
    i32::try_from(remaining).unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/netease_captcha_tests.rs"]
mod tests;
