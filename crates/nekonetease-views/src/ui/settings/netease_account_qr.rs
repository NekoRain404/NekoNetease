//! The QR sign-in flow: key acquisition, polling, and the rendered code.
//!
//! Kept beside [`super`] rather than in it because the polling loop and its cancellation
//! state owe nothing to the password or SMS forms.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nekonetease_app::cloud::QrStatus;
use nekonetease_app::state::AppState;
use nekonetease_core::error::describe;
use nekonetease_ui::{AppWindow, CloudAccount};
use parking_lot::Mutex;
use qrcode::QrCode;
use qrcode::render::svg;
use slint::{ComponentHandle, Image, Weak};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub(super) struct QrLoginController {
    generation: Arc<AtomicU64>,
    active: Arc<Mutex<Option<(u64, CancellationToken)>>>,
}

impl QrLoginController {
    pub(super) fn start(&self, weak: Weak<AppWindow>, state: AppState, automatic: bool) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let cancellation = CancellationToken::new();
        if let Some((_, previous)) = self.active.lock().replace((generation, cancellation.clone()))
        {
            previous.cancel();
        }
        self.paint(&weak, generation, 1, true, None, None);

        let controller = self.clone();
        state.runtime.spawn(async move {
            let key = tokio::select! {
                () = cancellation.cancelled() => return,
                result = async {
                    if automatic {
                        state.cloud_playback.begin_qr_login_if_signed_out().await
                    } else {
                        state.cloud_playback.begin_qr_login().await.map(Some)
                    }
                } => match result {
                    Ok(Some(key)) => key,
                    Ok(None) => {
                        controller.paint(&weak, generation, 0, false, None, None);
                        controller.finish(generation);
                        return;
                    }
                    Err(error) => {
                        if controller.is_current(generation) {
                            controller.paint(
                                &weak,
                                generation,
                                6,
                                false,
                                None,
                                Some(describe(&error)),
                            );
                        }
                        controller.finish(generation);
                        return;
                    }
                },
            };

            if !controller.is_current(generation) {
                return;
            }
            let svg = match login_qr_svg(&key) {
                Ok(svg) => svg,
                Err(error) => {
                    controller.paint(&weak, generation, 6, false, None, Some(error.to_string()));
                    controller.finish(generation);
                    return;
                }
            };
            controller.paint(&weak, generation, 2, false, Some(svg.clone()), None);

            loop {
                tokio::select! {
                    () = cancellation.cancelled() => return,
                    () = tokio::time::sleep(Duration::from_secs(3)) => {},
                }
                if !controller.is_current(generation) {
                    return;
                }

                let status = tokio::select! {
                    () = cancellation.cancelled() => return,
                    result = state.cloud_playback.check_qr_login(&key) => result,
                };
                match status {
                    Ok(QrStatus::Waiting) => {}
                    Ok(QrStatus::Confirming) => {
                        controller.paint(&weak, generation, 3, false, Some(svg.clone()), None);
                    }
                    Ok(QrStatus::Expired) => {
                        controller.paint(&weak, generation, 4, false, Some(svg), None);
                        controller.finish(generation);
                        return;
                    }
                    Ok(QrStatus::Authorized) => {
                        controller.paint(&weak, generation, 5, false, Some(svg), None);
                        controller.finish(generation);
                        return;
                    }
                    Err(error) => {
                        controller.paint(
                            &weak,
                            generation,
                            6,
                            false,
                            Some(svg),
                            Some(describe(&error)),
                        );
                        controller.finish(generation);
                        return;
                    }
                }
            }
        });
    }

    pub(super) fn cancel(&self, weak: &Weak<AppWindow>) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        if let Some((_, cancellation)) = self.active.lock().take() {
            cancellation.cancel();
        }
        self.paint(weak, generation, 0, false, None, None);
    }

    fn paint(
        &self,
        weak: &Weak<AppWindow>,
        generation: u64,
        status: i32,
        busy: bool,
        svg: Option<String>,
        notice: Option<String>,
    ) {
        let weak = weak.clone();
        let current_generation = self.generation.clone();
        let notice = notice.unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if current_generation.load(Ordering::Acquire) != generation {
                return;
            }
            let account = ui.global::<CloudAccount>();
            let image = svg
                .as_deref()
                .and_then(|svg| Image::load_from_svg_data(svg.as_bytes()).ok())
                .unwrap_or_default();
            account.set_qr_image(image);
            account.set_qr_status(status);
            account.set_qr_busy(busy);
            account.set_notice(notice.into());
        });
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    fn finish(&self, generation: u64) {
        let mut active = self.active.lock();
        if active.as_ref().is_some_and(|(current, _)| *current == generation) {
            active.take();
        }
    }
}

pub(super) fn login_qr_svg(key: &str) -> Result<String, qrcode::types::QrError> {
    let url = format!("https://music.163.com/login?codekey={key}");
    let code = QrCode::new(url.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#11111b"))
        .light_color(svg::Color("#ffffff"))
        .build())
}
