//! 原生收集诊断。命令行入口在单实例检查和图形窗口初始化之前运行。

pub const ARG: &str = "--collect-diagnostics";

#[cfg(windows)]
mod environment;

pub fn requested(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == ARG)
}

/// 应用内按钮启动一次独立的收集进程，当前岛仔可以继续运行。
pub fn launch() -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg(ARG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(windows)]
pub fn run(data_dir: &std::path::Path) -> Result<(), String> {
    archive::run(data_dir)
}

#[cfg(windows)]
mod archive {
    use super::environment;
    use regex::{Regex, RegexBuilder};
    use serde_json::{json, Map, Value};
    use std::ffi::OsStr;
    use std::fs;
    use std::io::{self, Cursor, Read, Write};
    use std::os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    };
    use std::path::{Path, PathBuf};
    use windows::core::{w, PCWSTR};
    use windows::Win32::{
        Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT},
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{
            FOLDERID_Desktop, ILCreateFromPathW, ILFree, SHGetKnownFolderPath,
            SHOpenFolderAndSelectItems, KF_FLAG_DEFAULT,
        },
        UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK},
    };
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    const OUTPUT_ARG: &str = "--diagnostics-output-dir";
    const MAX_LOG_SIZE: u64 = 2 * 1024 * 1024;
    const LOGS: &[&str] = &[
        "last-error.log",
        "last-graphics.log",
        "last-startup.log",
        "resources.log",
        "resources.previous.log",
        "last-resource-anomaly.log",
        "resource-guard.log",
        "weather-network.log",
        "weather-network.previous.log",
        "messenger-events.log",
        "messenger-events.previous.log",
        "wechat-events.log",
        "wechat-tray.log",
        "calendar.log",
        "calendar.previous.log",
        "lyrics.log",
        "lyrics.previous.log",
        "music-gaps.log",
        "music-gaps.previous.log",
        "frames.log",
        "frames.previous.log",
        "auto-restart.log",
        "glass-diagnostics.txt",
    ];

    pub(super) fn run(data_dir: &Path) -> Result<(), String> {
        let mut args = std::env::args_os().skip(1);
        let output = if args.any(|arg| arg == OUTPUT_ARG) {
            Some(PathBuf::from(
                args.next()
                    .filter(|arg| !arg.is_empty())
                    .ok_or("请指定诊断包保存目录")?,
            ))
        } else {
            None
        };
        let target = output
            .clone()
            .or_else(desktop)
            .unwrap_or_else(|| data_dir.to_owned());
        let private_paths: Vec<_> = ["LOCALAPPDATA", "APPDATA", "USERPROFILE"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (key, value)))
            .collect();
        let archive = save(data_dir, &target, environment::collect(), &private_paths)
            .map_err(|error| format!("无法保存诊断包：{error}"))?;
        if output.is_none() {
            reveal(&archive);
        }
        Ok(())
    }

    fn is_reparse_point(metadata: &fs::Metadata) -> bool {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    }

    fn read_text(path: &Path) -> io::Result<String> {
        // 只收集实际日志文件，避免从重解析点读取白名单以外的数据。
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || is_reparse_point(&metadata) {
            return Err(io::Error::other("不是普通文件"));
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)?;
        if is_reparse_point(&file.metadata()?) {
            return Err(io::Error::other("不是普通文件"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_LOG_SIZE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_LOG_SIZE {
            return Err(io::Error::other("日志超过收集大小上限"));
        }
        let text = String::from_utf8(bytes).map_err(io::Error::other)?;
        Ok(text
            .strip_prefix('\u{feff}')
            .map(str::to_owned)
            .unwrap_or(text))
    }

    fn path_masks(paths: &[(&str, String)]) -> Vec<(Regex, String)> {
        let mut masks = Vec::new();
        for (key, path) in paths.iter().filter(|(_, path)| !path.is_empty()) {
            let windows_path = path.replace('/', r"\");
            for spelling in [
                windows_path.replace('\\', r"\\"),
                windows_path.replace('\\', "/"),
                windows_path,
            ] {
                let regex = RegexBuilder::new(&regex::escape(&spelling))
                    .case_insensitive(true)
                    .build()
                    .expect("字面路径应能编译为正则表达式");
                masks.push((regex, format!("%{key}%")));
            }
        }
        masks
    }

    fn redact(mut text: String, masks: &[(Regex, String)]) -> String {
        for (pattern, replacement) in masks {
            if let std::borrow::Cow::Owned(rewritten) =
                pattern.replace_all(&text, replacement.as_str())
            {
                text = rewritten;
            }
        }
        text
    }

    fn settings(text: &str) -> (Vec<&str>, Vec<&str>) {
        use crate::app::BLUETOOTH_PAGE;
        use crate::model::{Dock, IslandShape, Skin};
        const PAGE_COUNT: usize = BLUETOOTH_PAGE + 1;
        let mut display = Vec::new();
        let mut features = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let flag = matches!(value, "0" | "1");
            let valid_display = match key {
                "skin" => Skin::ALL.iter().any(|skin| skin.key() == value) || value == "glass_dark",
                "island_shape" => IslandShape::ALL.iter().any(|shape| shape.key() == value),
                "dock" => Dock::ALL.iter().any(|dock| dock.key() == value),
                "settings_theme" => matches!(value, "system" | "light" | "dark"),
                "island_size" => value.parse::<u32>().is_ok(),
                "island_offset" => value.parse::<i32>().is_ok(),
                "glass_blur" | "minimal" | "pet" => flag,
                _ => false,
            };
            let valid_feature = match key {
                "notifications"
                | "wechat_notifications"
                | "wechat_tray_open"
                | "wecom_notifications"
                | "feishu_notifications"
                | "dingtalk_notifications"
                | "read_notifications"
                | "qq_only"
                | "lyrics_enabled"
                | "island_lyrics"
                | "render_compat"
                | "calendar_auto_sync"
                | "mail_enabled" => flag,
                "visible_pages" => {
                    value.len() <= PAGE_COUNT
                        && value.bytes().all(|byte| matches!(byte, b'0' | b'1'))
                }
                "page_order" => {
                    let mut order = value.split(',');
                    order.clone().count() <= PAGE_COUNT
                        && order.all(|index| {
                            index.bytes().all(|byte| byte.is_ascii_digit())
                                && index.parse::<usize>().is_ok_and(|page| page < PAGE_COUNT)
                        })
                }
                _ => false,
            };
            if valid_display {
                display.push(line);
            }
            if valid_feature {
                features.push(line);
            }
        }
        (display, features)
    }

    fn save(
        data_dir: &Path,
        output_dir: &Path,
        mut info: Map<String, Value>,
        private_paths: &[(&str, String)],
    ) -> io::Result<PathBuf> {
        let time = chrono::Local::now();
        let diagnostics = data_dir.join("diagnostics");
        let diagnostics_is_plain = fs::symlink_metadata(&diagnostics)
            .is_ok_and(|metadata| metadata.is_dir() && !is_reparse_point(&metadata));
        let masks = path_masks(private_paths);
        let mut missing = Vec::new();
        let mut zip = ZipWriter::new(Cursor::new(Vec::<u8>::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for name in LOGS {
            let source = if *name == "glass-diagnostics.txt" {
                data_dir.join(name)
            } else if diagnostics_is_plain {
                diagnostics.join(name)
            } else {
                missing.push((*name).to_owned());
                continue;
            };
            match read_text(&source) {
                Ok(log) => {
                    zip.start_file(name, options)?;
                    zip.write_all(redact(log, &masks).as_bytes())?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing.push((*name).to_owned())
                }
                Err(_) => missing.push(format!("{name}：无法读取")),
            }
        }
        info.insert("CollectedAt".into(), json!(time.to_rfc3339()));
        info.insert("MissingLogs".into(), json!(missing));
        if let Ok(text) = read_text(&data_dir.join("settings.txt")) {
            let (display, features) = settings(&text);
            info.insert("DisplaySettings".into(), json!(display));
            info.insert("FeatureSettings".into(), json!(features));
        }
        zip.start_file("environment.json", options)?;
        let environment = redact(serde_json::to_string_pretty(&info)?, &masks);
        zip.write_all(environment.as_bytes())?;
        let bytes = zip.finish()?.into_inner();

        fs::create_dir_all(output_dir)?;
        let output = output_dir.join(format!(
            "岛仔诊断-{}-{}.zip",
            time.format("%Y%m%d-%H%M%S-%9f"),
            std::process::id()
        ));
        crate::model::atomic_write(&output, &bytes)?;
        Ok(output)
    }

    fn desktop() -> Option<PathBuf> {
        unsafe {
            let wide = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
            let path = wide.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(wide.0.cast()));
            path
        }
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    fn reveal(path: &Path) {
        let message = format!(
            "诊断包已保存到：\n{}\n\n诊断包包含岛仔日志和系统状态。音乐日志可能包含歌名和歌手，请自行决定是否分享。\n\n点击确定后查看诊断包。",
            path.display()
        );
        let message_wide = wide(OsStr::new(&message));
        let path_wide = wide(path.as_os_str());
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message_wide.as_ptr()),
                w!("岛仔诊断"),
                MB_OK | MB_ICONINFORMATION,
            );
            let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let item = ILCreateFromPathW(PCWSTR(path_wide.as_ptr()));
            if !item.is_null() {
                let _ = SHOpenFolderAndSelectItems(item, None, 0);
                ILFree(Some(item));
            }
            if initialized {
                CoUninitialize();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn archive_keeps_allowed_logs_and_redacts_paths_without_reading_credentials() {
            let root = std::env::temp_dir().join(format!(
                "daozai-diagnostics-fixture-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
            let diag = root.join("diagnostics");
            fs::create_dir_all(&diag).unwrap();
            let account_dir = r"C:\Users\Fixture";
            let raw_path = r"c:\users\fixture\Player\player.exe";
            fs::write(
                diag.join("resources.log"),
                json!({"Path":raw_path}).to_string(),
            )
            .unwrap();
            fs::write(diag.join("frames.log"), raw_path).unwrap();
            fs::write(diag.join("frames.previous.log"), "previous").unwrap();
            fs::write(diag.join("music-gaps.log"), "missing album").unwrap();
            fs::write(diag.join("credentials.json"), "synthetic-oauth-token").unwrap();
            fs::write(root.join("settings.txt"), "dock=left\nlyrics_enabled=1\nnotifications=some-secret\nemail=private@example.test\nvisible_pages=11111111101\npage_order=10,0,1,2,3,4,5,6,7,8,9\npage_order=11\nvisible_pages=111111111111").unwrap();

            let snapshot = Map::from_iter([("Path".into(), json!(raw_path))]);
            let masks = [("USERPROFILE", account_dir.into())];
            assert!(save(&root, &diag.join("frames.log"), snapshot.clone(), &masks).is_err());
            let output = root.join("out");
            let archive = save(&root, &output, snapshot, &masks).unwrap();
            assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
            {
                let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
                let mut text = |name: &str| {
                    let mut body = String::new();
                    zip.by_name(name)
                        .unwrap()
                        .read_to_string(&mut body)
                        .unwrap();
                    body
                };
                let resources: Value = serde_json::from_str(&text("resources.log")).unwrap();
                assert_eq!(resources["Path"], r"%USERPROFILE%\Player\player.exe");
                assert_eq!(text("frames.log"), r"%USERPROFILE%\Player\player.exe");
                assert_eq!(text("frames.previous.log"), "previous");
                assert_eq!(text("music-gaps.log"), "missing album");
                let environment: Value = serde_json::from_str(&text("environment.json")).unwrap();
                assert_eq!(environment["Path"], r"%USERPROFILE%\Player\player.exe");
                assert_eq!(environment["DisplaySettings"], json!(["dock=left"]));
                assert_eq!(
                    environment["FeatureSettings"],
                    json!([
                        "lyrics_enabled=1",
                        "visible_pages=11111111101",
                        "page_order=10,0,1,2,3,4,5,6,7,8,9"
                    ])
                );
                assert!(environment["MissingLogs"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("lyrics.log")));
                assert!(zip.by_name("credentials.json").is_err());
                assert!(zip.by_name("settings.txt").is_err());
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_explicit_switch_starts_collection() {
        assert!(requested(
            ["daozai.exe".to_string(), ARG.to_string()].into_iter()
        ));
        assert!(!requested(
            ["daozai.exe".to_string(), "--settings".to_string()].into_iter()
        ));
    }
}
