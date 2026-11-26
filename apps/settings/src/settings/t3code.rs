//! T3 Code integration page in Settings: install/update from EvilBorsch/t3code
//! (`main`), pick the default harness (provider) and model, and configure
//! voice dictation & Dynamic Island triggers.

use denial_look_gpui::{Icon, island, solid};
use gpui::{Context, Div, SharedString, Task, div, prelude::*};
use serde_json::{Value, json};

use super::parts::{ROW_GAP, ROW_PAD, TILE};
use super::{Layer, Settings};
use crate::catalog::Section;
use crate::denial;
use crate::t3code::{self, ProviderInfo, Status};

#[derive(Default)]
pub(super) struct T3Code {
    pub(super) status: Status,
    pub(super) read: bool,
    pub(super) busy: bool,
    pub(super) notice: Option<String>,
    pub(super) error: Option<String>,
    reading: Option<Task<()>>,
}

impl Settings {
    pub(super) fn follow_t3code(&mut self, cx: &mut Context<Self>) {
        if !self.screen.is_page("t3code") && !matches!(self.screen, super::Screen::Home) {
            self.t3code.reading = None;
            return;
        }
        if self.t3code.read && !self.screen.is_page("t3code") {
            return;
        }
        self.refresh_t3code(cx);
    }

    pub(super) fn refresh_t3code(&mut self, cx: &mut Context<Self>) {
        let override_dir = self
            .kept("t3Code.installDir")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.t3code.reading = Some(cx.spawn(async move |this, cx| {
            let status = cx
                .background_spawn(async move { t3code::probe(override_dir.as_deref()) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.t3code.status = status;
                view.t3code.read = true;
                cx.notify();
            });
        }));
    }

    pub(super) fn t3code_summary(&self) -> Option<SharedString> {
        if !self.t3code.read {
            return None;
        }
        let s = &self.t3code.status;
        if !s.installed {
            return Some("Не установлен".into());
        }
        let run = if s.server_running {
            "запущен"
        } else {
            "остановлен"
        };
        let label = match s.commit.as_deref() {
            Some(c) => format!("{c} · {run}"),
            None => format!("Установлен · {run}"),
        };
        Some(label.into())
    }

    pub(super) fn t3code_page(
        &self,
        sections: &'static [Section],
        layer: Layer,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut parts = self.page(sections, layer, cx);
        let status = &self.t3code.status;

        // 1. Installation & updates island.
        parts.push(self.chip_row("Установка и обновления"));
        let install_detail: SharedString = if status.installed {
            let commit = status.commit.as_deref().unwrap_or("main");
            let run = if status.server_running {
                "сервер запущен (127.0.0.1:3773)"
            } else {
                "сервер не запущен"
            };
            format!("EvilBorsch/t3code ({commit}) · {run}").into()
        } else {
            "EvilBorsch/t3code (ветка main) не установлен".into()
        };

        let mut install_card = island(&self.look).child(self.row(
            "t3code.status",
            Some(Icon::Code),
            if status.installed {
                "T3 Code установлен"
            } else {
                "T3 Code не установлен"
            },
            Some(install_detail),
            (true, false),
        ));

        let action_title: &'static str = if self.t3code.busy {
            "Выполняется сборка T3 Code..."
        } else if status.installed {
            "Обновить T3 Code из main"
        } else {
            "Установить T3 Code из main"
        };
        let action_detail: SharedString = if let Some(err) = &self.t3code.error {
            format!("Ошибка: {err}").into()
        } else if let Some(msg) = &self.t3code.notice {
            msg.clone().into()
        } else if status.installed {
            "Подтянет свежий main из github.com/EvilBorsch/t3code и пересоберёт".into()
        } else {
            "Клонирует github.com/EvilBorsch/t3code (main), соберёт и добавит ярлык".into()
        };
        let action_row = self.row(
            "t3code.install-or-update",
            Some(Icon::Update),
            action_title,
            Some(action_detail),
            (false, !status.installed),
        );
        let action_row = self.live(
            action_row,
            layer.live() && !self.t3code.busy,
            cx,
            |this, _, cx| this.run_t3code_install_or_update(cx),
        );
        install_card = install_card
            .child(self.separator(0.))
            .child(action_row);

        if status.installed {
            let open_row = self.row(
                "t3code.launch",
                Some(Icon::Apps),
                "Открыть T3 Code",
                Some("Запустить приложение и сервер T3 Code".into()),
                (false, true),
            );
            let open_row = self.live(open_row, layer.live(), cx, |this, _, cx| {
                let dir = this
                    .kept("t3Code.installDir")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                match t3code::launch(dir.as_deref()) {
                    Ok(()) => {
                        this.t3code.notice = Some("T3 Code запускается...".to_owned());
                        this.t3code.error = None;
                    }
                    Err(e) => {
                        this.t3code.error = Some(e);
                    }
                }
                cx.notify();
            });
            install_card = install_card
                .child(self.separator(0.))
                .child(open_row);
        }
        parts.push(install_card);

        // 2. Harness (provider instance) picker.
        let selected_instance = self
            .kept("t3Code.providerInstanceId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let selected_model = self
            .kept("t3Code.model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        let providers = &status.providers;
        parts.push(self.chip_row("Harness (провайдер агента)"));
        if providers.is_empty() {
            parts.push(
                island(&self.look).child(self.row(
                    "t3code.no-providers",
                    Some(Icon::Info),
                    "Провайдеры пока не обнаружены",
                    Some("Откройте T3 Code и войдите хотя бы в один провайдер".into()),
                    (true, true),
                )),
            );
        } else {
            let mut card = island(&self.look);
            let total = providers.len();
            for (ix, provider) in providers.iter().enumerate() {
                if ix > 0 {
                    card = card.child(self.separator(0.));
                }
                let is_selected = (!selected_instance.is_empty()
                    && selected_instance == provider.instance_id)
                    || (selected_instance.is_empty() && ix == 0);
                let detail: SharedString = format!(
                    "{} · {} моделей",
                    if provider.ready {
                        "готов"
                    } else if provider.enabled {
                        "включён"
                    } else {
                        "выключен"
                    },
                    provider.models.len()
                )
                .into();
                let mut row = self.row(
                    ("t3code-provider", ix),
                    None,
                    provider.display_name.clone(),
                    Some(detail),
                    (ix == 0, ix + 1 == total),
                );
                if is_selected {
                    row = row.child(
                        div()
                            .flex_none()
                            .text_color(solid(self.look.accent))
                            .child("✓"),
                    );
                }
                let inst_id = provider.instance_id.clone();
                let driver = provider.driver.clone();
                let default_model = provider
                    .models
                    .iter()
                    .find(|m| m.is_default)
                    .or_else(|| provider.models.first())
                    .map(|m| m.slug.clone())
                    .unwrap_or_default();
                let row = self.live(row, layer.live(), cx, move |this, _, cx| {
                    let inst = inst_id.clone();
                    let drv = driver.clone();
                    let mdl = default_model.clone();
                    this.change(
                        "Harness",
                        move |doc| {
                            denial::put(doc, "t3Code.providerInstanceId", json!(inst));
                            denial::put(doc, "t3Code.providerDriver", json!(drv));
                            denial::put(doc, "t3Code.model", json!(mdl));
                        },
                        cx,
                    );
                });
                card = card.child(row);
            }
            parts.push(card);
        }

        // 3. Model picker for the active harness.
        let active_provider: Option<&ProviderInfo> = providers
            .iter()
            .find(|p| p.instance_id == selected_instance)
            .or_else(|| providers.first());

        if let Some(provider) = active_provider
            && !provider.models.is_empty()
        {
            parts.push(self.chip_row("Модель"));
            let mut card = island(&self.look);
            let total = provider.models.len();
            for (ix, model) in provider.models.iter().enumerate() {
                if ix > 0 {
                    card = card.child(self.separator(0.));
                }
                let is_selected = (!selected_model.is_empty() && selected_model == model.slug)
                    || (selected_model.is_empty() && model.is_default);
                let mut row = self.row(
                    ("t3code-model", ix),
                    None,
                    model.name.clone(),
                    Some(model.slug.clone().into()),
                    (ix == 0, ix + 1 == total),
                );
                if is_selected {
                    row = row.child(
                        div()
                            .flex_none()
                            .text_color(solid(self.look.accent))
                            .child("✓"),
                    );
                }
                let slug = model.slug.clone();
                let inst = provider.instance_id.clone();
                let drv = provider.driver.clone();
                let row = self.live(row, layer.live(), cx, move |this, _, cx| {
                    let s = slug.clone();
                    let i = inst.clone();
                    let d = drv.clone();
                    this.change(
                        "Модель",
                        move |doc| {
                            denial::put(doc, "t3Code.providerInstanceId", json!(i));
                            denial::put(doc, "t3Code.providerDriver", json!(d));
                            denial::put(doc, "t3Code.model", json!(s));
                        },
                        cx,
                    );
                });
                card = card.child(row);
            }
            parts.push(card);
        }

        // 4. Voice & Workspace summary island.
        parts.push(self.chip_row("Голос и рабочий каталог"));
        let voice_detail: SharedString = if status.gigaam_installed {
            "GigaAM v3 (русский) загружена и готова".into()
        } else {
            "Модель GigaAM ещё не скачана (скачается через T3 Code → Голос)".into()
        };
        parts.push(
            island(&self.look)
                .child(self.row(
                    "t3code.voice-model",
                    Some(Icon::Speaker),
                    "Распознавание речи",
                    Some(voice_detail),
                    (true, false),
                ))
                .child(self.separator(0.))
                .child(self.row(
                    "t3code.workspace",
                    Some(Icon::Storage),
                    "Каталог тредов и права",
                    Some("~ (домашняя папка) · full-access (bypass permissions)".into()),
                    (false, true),
                )),
        );

        let _ = (ROW_GAP, ROW_PAD, TILE);
        parts
    }

    fn run_t3code_install_or_update(&mut self, cx: &mut Context<Self>) {
        if self.t3code.busy {
            return;
        }
        let override_dir = self
            .kept("t3Code.installDir")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let repo_url = self
            .kept("t3Code.repoUrl")
            .and_then(Value::as_str)
            .unwrap_or(t3code::DEFAULT_REPO_URL)
            .to_owned();
        let branch = self
            .kept("t3Code.branch")
            .and_then(Value::as_str)
            .unwrap_or(t3code::DEFAULT_BRANCH)
            .to_owned();

        self.t3code.busy = true;
        self.t3code.error = None;
        self.t3code.notice = Some("Клонирование и сборка EvilBorsch/t3code...".to_owned());
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    t3code::install_or_update(override_dir.as_deref(), &repo_url, &branch)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.t3code.busy = false;
                match result {
                    Ok(commit) => {
                        view.t3code.notice = Some(format!("Готово (коммит {commit})"));
                        view.t3code.error = None;
                        view.refresh_t3code(cx);
                    }
                    Err(err) => {
                        view.t3code.error = Some(err);
                        view.t3code.notice = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
