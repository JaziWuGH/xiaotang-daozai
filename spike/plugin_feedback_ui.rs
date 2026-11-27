//! 插件详情和已安装列表共用的反馈入口。
use eframe::egui;

const PRIVATE_REPORT: &str =
    "https://github.com/Renly1994/xiaotang-daozai-plugins/security/advisories/new";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Topic {
    #[default]
    Plugin,
    Install,
    Private,
}

impl Topic {
    fn title(self) -> &'static str {
        match self {
            Self::Plugin => "问题或建议",
            Self::Install => "安装与更新",
            Self::Private => "安全或隐私",
        }
    }
}

#[derive(Clone)]
struct Feedback {
    id: String,
    name: String,
    version: String,
    topic: Topic,
    copied: bool,
}

impl Feedback {
    fn title(&self) -> String {
        format!("[{}] {}", self.topic.title(), self.name)
    }

    fn draft(&self) -> String {
        format!(
            "插件：{}\n插件 ID：{}\n插件版本：{}\n岛仔版本：{}\n\n## {}\n\n## 复现步骤\n",
            self.name,
            self.id,
            self.version,
            env!("CARGO_PKG_VERSION"),
            self.topic.title(),
        )
    }

    fn form_url(&self) -> url::Url {
        if self.topic == Topic::Private {
            return url::Url::parse(PRIVATE_REPORT).expect("私密报告地址有效");
        }
        let mut url =
            url::Url::parse("https://github.com/Renly1994/xiaotang-daozai-plugins/issues/new")
                .expect("插件反馈地址有效");
        url.query_pairs_mut()
            .append_pair("title", &self.title())
            .append_pair("body", &self.draft());
        url
    }
}

pub(super) fn request_plugin_feedback(ui: &egui::Ui, id: &str, name: &str, version: &str) {
    let feedback = Feedback {
        id: id.to_owned(),
        name: name.split_whitespace().collect::<Vec<_>>().join(" "),
        version: version.to_owned(),
        topic: Topic::Plugin,
        copied: false,
    };
    ui.data_mut(|data| data.insert_temp(egui::Id::new("plugin-feedback"), feedback));
    ui.ctx().request_repaint();
}

/// 插件反馈弹窗：三种反馈各是一张可点的选项卡，点了直接去对应入口；
/// 没有 GitHub 账号时可以复制反馈信息转交。
pub(super) fn plugin_feedback(ui: &egui::Ui) {
    use crate::extras::{settings_apply_style, settings_hint, settings_modal_frame, settings_modal_title};
    let id = egui::Id::new("plugin-feedback");
    let Some(mut feedback) = ui.data(|data| data.get_temp::<Feedback>(id)) else {
        return;
    };
    let ctx = ui.ctx().clone();
    let mut done = false;
    let modal = egui::Modal::new(id.with("dialog"))
        .frame(settings_modal_frame(&ctx))
        .backdrop_color(egui::Color32::from_black_alpha(110))
        .show(&ctx, |ui| {
            ui.set_width(360.0);
            settings_apply_style(ui);
            let mut close = settings_modal_title(ui, "插件反馈");
            settings_hint(
                ui,
                &format!(
                    "{} · v{}，插件 ID 和岛仔版本会自动填好。",
                    feedback.name, feedback.version
                ),
            );
            ui.add_space(4.0);
            for (topic, detail) in [
                (
                    Topic::Plugin,
                    "在 GitHub 补充使用场景或复现步骤，提交后通知插件维护者。问题单公开可见。",
                ),
                (Topic::Install, "装不上、更新失败，由岛仔维护者跟进。"),
                (Topic::Private, "复制插件信息并打开私密报告，只有维护者能看到。"),
            ] {
                if super::option_card(ui, topic.title(), detail, false) {
                    feedback.topic = topic;
                    if topic == Topic::Private {
                        ui.ctx().copy_text(feedback.draft());
                    }
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(feedback.form_url().to_string()));
                    done = true;
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let copy = if feedback.copied {
                    "已复制反馈信息"
                } else {
                    "没有 GitHub？复制反馈信息"
                };
                if ui.add(egui::Button::new(copy).frame(false)).clicked() {
                    feedback.topic = Topic::Plugin;
                    ui.ctx().copy_text(feedback.draft());
                    feedback.copied = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= ui.button("关闭").clicked();
                });
            });
            close
        });
    let close = done || modal.inner || modal.should_close();
    ui.data_mut(|data| {
        if close {
            data.remove::<Feedback>(id);
        } else {
            data.insert_temp(id, feedback);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn issue_draft_preserves_plugin_context_in_query_values() {
        let feedback = Feedback {
            id: "example.payday".into(),
            name: "发薪 & 旅行".into(),
            version: "1.0.1".into(),
            topic: Topic::Install,
            copied: false,
        };
        let url = feedback.form_url();
        assert_eq!(url.host_str(), Some("github.com"));
        assert!(url.path().ends_with("/xiaotang-daozai-plugins/issues/new"));
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.get("title").unwrap(), "[安装与更新] 发薪 & 旅行");
        assert_eq!(query.get("body").unwrap().as_ref(), feedback.draft());
        assert!(feedback.draft().contains("插件 ID：example.payday"));
    }

    #[test]
    fn private_feedback_keeps_draft_out_of_url() {
        let feedback = Feedback {
            id: "example.payday".into(),
            name: "发薪倒计时".into(),
            version: "1.0.1".into(),
            topic: Topic::Private,
            copied: false,
        };
        let form = feedback.form_url();
        assert_eq!(form.host_str(), Some("github.com"));
        assert!(form.path().ends_with("/security/advisories/new"));
        assert!(form.query().is_none());
        assert!(feedback
            .draft()
            .starts_with("插件：发薪倒计时\n插件 ID：example.payday\n插件版本：1.0.1\n"));
        assert!(feedback.draft().contains("## 安全或隐私"));
    }
}
