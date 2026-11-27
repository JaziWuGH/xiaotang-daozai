//! 通过已连接的官方 CLI 获取额度，复用岛仔的 AI 额度面板。
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod cli;
#[cfg(any(windows, test))]
mod parse;

pub(crate) const INSTALL_URL: &str = "https://antigravity.google/docs/getting-started?tab=cli";
pub(crate) const CLI_MISSING: &str = "请先安装 Antigravity CLI";
pub(crate) const CLI_OLD: &str = "请更新 Antigravity CLI 后再连接";
pub(crate) const CLI_UNTRUSTED: &str = "无法验证 Antigravity CLI 的 Google 签名，请从官网重新安装";
pub(crate) const LOGIN_REQUIRED: &str = "请在 Antigravity CLI 中登录";
pub(crate) const SLASH_DISABLED: &str = "请在 Antigravity CLI 启用斜杠命令后刷新";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Workspace,
    WorkspaceDenied,
    Enterprise,
    Cloud,
    ApiKey,
}

impl Mode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace 暂无可读取额度",
            Self::WorkspaceDenied => "组织未开放访问",
            Self::Enterprise => "企业共享额度",
            Self::Cloud => "Google Cloud 按量计费",
            Self::ApiKey => "API Key 模式",
        }
    }

    pub(crate) fn url(self) -> Option<&'static str> {
        match self {
            Self::Workspace | Self::WorkspaceDenied => {
                Some("https://antigravity.google/docs/workspace")
            }
            Self::Enterprise => Some("https://console.cloud.google.com/gemini-enterprise"),
            Self::Cloud => Some("https://console.cloud.google.com/billing"),
            Self::ApiKey => Some("https://aistudio.google.com/usage"),
        }
    }
}

#[cfg(windows)]
fn lock_official_cli(exe: &Path) -> Option<std::fs::File> {
    crate::messenger::trust::lock_signed_executable(exe, |publisher| {
        publisher.eq_ignore_ascii_case("Google LLC")
    })
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct UnavailableWindow {
    pub label: String,
    pub reset: Option<i64>,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Details {
    pub credits: Option<f64>,
    #[serde(default)]
    pub credits_at: Option<i64>,
    pub unavailable: Vec<UnavailableWindow>,
    pub mode: Option<Mode>,
}

#[cfg(windows)]
enum Failure {
    Message(String),
    Login,
    UnsupportedMode(Mode),
    Temporary(String),
}

pub(super) fn discover() -> Option<PathBuf> {
    #[cfg(windows)]
    return cli::discover();
    #[cfg(not(windows))]
    None
}

pub(super) fn fetch(source: &Path) -> Result<super::Snapshot, (String, u64)> {
    #[cfg(windows)]
    {
        let exe = discover().ok_or((CLI_MISSING.into(), 0))?;
        if source != exe {
            return Err(("Antigravity CLI 位置已变化，请重新连接".into(), 0));
        }
        let exe = exe.canonicalize().map_err(|_| (CLI_MISSING.into(), 0))?;
        let _verified_file = lock_official_cli(&exe).ok_or((CLI_UNTRUSTED.into(), 0))?;
        let mut snapshot = super::Snapshot {
            plan: "当前 CLI 账号".into(),
            ..Default::default()
        };
        let usage_report = cli::report(&exe, false);
        snapshot.at = chrono::Utc::now().timestamp();
        let details = match usage_report {
            Ok(json) => {
                let usage = parse::usage(&json).map_err(|error| (error, 60))?;
                snapshot.windows = usage.windows;
                // 两个报告都不含账号 ID，单独记录积分的查询时间，不将其视为同一时刻的读数。
                let credits = match cli::report(&exe, true) {
                    Ok(json) => parse::credits(&json),
                    Err(Failure::Login | Failure::UnsupportedMode(_)) => {
                        return Err(("CLI 登录状态已变化，请确认当前账号后刷新".into(), 0))
                    }
                    Err(_) => None,
                };
                Details {
                    credits,
                    credits_at: credits.map(|_| chrono::Utc::now().timestamp()),
                    unavailable: usage.unavailable,
                    mode: None,
                }
            }
            Err(Failure::UnsupportedMode(mode)) => Details {
                mode: Some(mode),
                ..Default::default()
            },
            Err(Failure::Login) => return Err((LOGIN_REQUIRED.into(), 120)),
            Err(Failure::Message(error)) => return Err((error, 0)),
            Err(Failure::Temporary(error)) => return Err((error, 60)),
        };
        snapshot.antigravity = Some(details);
        Ok(snapshot)
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        Err((CLI_MISSING.into(), 0))
    }
}
