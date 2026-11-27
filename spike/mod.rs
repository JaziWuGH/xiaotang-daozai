//! 独立验证入口；仅在显式打开运行环境 feature 时编译。
mod engines;
#[cfg(feature="sdk-js")] pub(crate) mod local;
#[cfg(all(windows, feature="sdk-js", feature="sdk-material"))]
pub use local::LocalPlugins;
mod ui;
use chrono::{Datelike, Local, NaiveDate};
use engines::{Bridge, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};
pub use ui::{live_card, live_settings};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn fail(text: &str) -> Box<dyn std::error::Error> {
    text.to_owned().into()
}

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    title: String,
    target: String,
    network: Vec<String>,
}
fn plugins() -> PathBuf {
    env::var_os("DAOZAI_SPIKE_PLUGINS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("spike/plugins"))
}
fn data() -> PathBuf {
    env::var_os("DAOZAI_SPIKE_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("daozai-sdk-spike"))
}
fn plugin_bytes(kind: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(plugins().join(match kind {
        "js" => "countdown.js",
        "wasm" => "countdown.wasm",
        _ => return Err(fail("未知运行环境")),
    }))?
    .take(256 * 1024 + 1)
    .read_to_end(&mut bytes)?;
    Ok(bytes)
}
struct Session {
    settings: Settings,
    engine: Engine,
    bridge: Rc<Bridge>,
    dir: PathBuf,
    days: i32,
    previous: Option<i32>,
}
impl Session {
    fn new(kind: &str, dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        let mut settings: Settings =
            serde_json::from_slice(&fs::read(plugins().join("settings.json"))?)?;
        let config = dir.join("settings.json");
        if config.exists() {
            let (title, target) = serde_json::from_slice(&fs::read(config)?)?;
            settings.title = title;
            settings.target = target;
        }
        let bridge = Rc::new(Bridge::new(settings.network.clone()));
        let saved = dir.join("last-days.json");
        if saved.exists() {
            bridge
                .previous
                .set(serde_json::from_slice(&fs::read(saved)?)?)
        }
        let engine = Engine::load(kind, &plugin_bytes(kind)?, bridge.clone())?;
        Ok(Self {
            settings,
            engine,
            bridge,
            dir,
            days: 0,
            previous: None,
        })
    }
    fn refresh(&mut self) -> Result<()> {
        let target = NaiveDate::parse_from_str(&self.settings.target, "%Y-%m-%d")?;
        self.bridge.pending.set(None);
        let last = self.engine.call(
            target.num_days_from_ce(),
            Local::now().date_naive().num_days_from_ce(),
            None,
        )?;
        let value = self
            .bridge
            .pending
            .take()
            .ok_or_else(|| fail("插件没有提交天数"))?;
        if value != self.bridge.previous.get() {
            crate::model::atomic_write(
                &self.dir.join("last-days.json"),
                serde_json::to_string(&value)?.as_bytes(),
            )?;
        }
        self.days = value;
        self.previous = (last != i32::MIN).then_some(last);
        self.bridge.previous.set(value);
        Ok(())
    }
    fn save(&mut self, title: String, target: String) -> Result<()> {
        if title.trim().is_empty() || title.chars().count() > 32 {
            return Err(fail("标题需为 1–32 字"));
        }
        NaiveDate::parse_from_str(&target, "%Y-%m-%d")?;
        let bytes = serde_json::to_vec(&(&title, &target))?;
        crate::model::atomic_write(&self.dir.join("settings.json"), &bytes)?;
        self.settings.title = title;
        self.settings.target = target;
        self.refresh()
    }
}

fn fault_report(s: &mut Session, kind: &str) -> Result<serde_json::Value> {
    let mut probes = Vec::new();
    for (id, name) in [
        (1, "死循环"),
        (2, "异常"),
        (3, "内存上限"),
        (4, "未声明联网"),
        (6, "已声明模拟传输"),
    ] {
        let before = s.bridge.dispatched.get();
        let start = Instant::now();
        let value = s.engine.call(0, 0, Some(id));
        let us = start.elapsed().as_secs_f64() * 1e6;
        let ok = match id {
            4 => value.as_ref().ok() == Some(&(-1)) && s.bridge.dispatched.get() == before,
            6 => value.as_ref().ok() == Some(&204) && s.bridge.dispatched.get() == before + 1,
            _ => value.is_err(),
        };
        probes.push(json!({"probe":name,"us":us,"observed":format!("{value:?}"),"pass":ok}));
        if !ok {
            return Err(fail(name));
        }
        s.refresh()?;
    }
    let bad = if kind == "js" {
        b"require('fs').readFileSync('sample.txt')".to_vec()
    } else {
        // 一个合法模块，只导入 WASI 文件函数；链接应在执行前被拒绝。
        let mut b = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0".to_vec();
        let mut imports = vec![1, 22];
        imports.extend(b"wasi_snapshot_preview1");
        imports.push(9);
        imports.extend(b"path_open");
        imports.extend([0, 0]);
        b.extend([2, imports.len() as u8]);
        b.extend(imports);
        b
    };
    let result = Engine::load(kind, &bad, s.bridge.clone())
        .err()
        .ok_or_else(|| fail("意外允许文件接口"))?;
    probes.push(json!({"probe":"直接文件访问","observed":result.to_string(),"pass":true}));
    Ok(json!(probes))
}

fn execute(args: &[String]) -> Result<()> {
    let kind = args.get(2).ok_or_else(|| fail("需要 js 或 wasm"))?;
    let mode = args.get(3).map(String::as_str).unwrap_or("ui");
    let root = data().join(kind);
    let report = match mode {
        "ui" => return ui::window(kind, None),
        "screenshot" => {
            return ui::window(
                kind,
                Some(PathBuf::from(
                    args.get(4).ok_or_else(|| fail("缺少图片路径"))?,
                )),
            )
        }
        "load" => {
            let mut rows = Vec::new();
            for n in [1, 10] {
                for repeat in 0..21 {
                    let start = Instant::now();
                    let mut instances = Vec::new();
                    for i in 0..n {
                        let mut s = Session::new(kind, root.join(format!("load-{i}")))?;
                        s.refresh()?;
                        instances.push(s)
                    }
                    rows.push(
                        json!({"n":n,"repeat":repeat,"ms":start.elapsed().as_secs_f64()*1000.0}),
                    );
                }
            }
            json!(rows)
        }
        "steady" => {
            let mut s = Session::new(kind, root.join("steady"))?;
            for _ in 0..35 {
                s.refresh()?;
                std::thread::sleep(Duration::from_secs(1));
            }
            json!({"refresh":35})
        }
        "fault" => {
            let mut s = Session::new(kind, root.join("fault"))?;
            fault_report(&mut s, kind)?
        }
        "once" => {
            let mut s = Session::new(kind, root.join("one"))?;
            if let (Some(title), Some(target)) = (args.get(5), args.get(6)) {
                s.save(title.clone(), target.clone())?
            } else {
                s.refresh()?
            }
            json!({"title":s.settings.title,"target":s.settings.target,"days":s.days,"previous":s.previous})
        }
        _ => return Err(fail("未知验证模式")),
    };
    fs::write(
        Path::new(args.get(4).ok_or_else(|| fail("缺少报告路径"))?),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

pub fn command() -> Option<std::process::ExitCode> {
    #[cfg(all(windows, feature = "sdk-js"))]
    if let Some(code) = local::market::author_command(&env::args_os().collect::<Vec<_>>()) {
        return Some(code);
    }
    let args: Vec<_> = env::args().collect();
    if args.get(1).map(String::as_str) != Some("--sdk-spike") {
        return None;
    }
    Some(match execute(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    })
}
