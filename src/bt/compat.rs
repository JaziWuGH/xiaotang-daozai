//! 宿主内置的接收器识别表与插件集市提供的扩展数据。
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const PACKAGE_ID: &str = "daozai.wireless-receivers";
const API_VERSION: u32 = 1;
#[cfg(windows)]
const BUILTIN: &[u8] = include_bytes!("../../assets/builtin-usb-receivers.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    api_version: u32,
    version: String,
    receivers: Vec<Receiver>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receiver {
    vid: String,
    pid: String,
    name: String,
}

pub(crate) struct Package {
    version: String,
    table: ReceiverTable,
}

impl Package {
    pub(crate) fn parse(raw: &[u8]) -> Result<Self, &'static str> {
        let manifest: Manifest = serde_json::from_slice(raw).map_err(|_| "兼容包格式无效")?;
        if manifest.id != PACKAGE_ID {
            return Err("此文件不是岛仔无线设备兼容包");
        }
        if manifest.api_version != API_VERSION {
            return Err("当前岛仔版本不支持此兼容包接口");
        }
        if !valid_text(&manifest.version, 32) {
            return Err("兼容包版本无效");
        }
        Ok(Self {
            version: manifest.version,
            table: ReceiverTable::new(manifest.receivers)?,
        })
    }

    #[cfg(any(test, all(windows, feature = "sdk-js")))]
    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    #[cfg(all(windows, feature = "sdk-js"))]
    pub(crate) fn receivers(&self) -> impl ExactSizeIterator<Item = ((u16, u16), &str)> {
        self.table.0.iter().map(|(&id, name)| (id, name.as_str()))
    }

    pub(super) fn receiver_name(&self, hardware: &str) -> Option<&str> {
        self.table.receiver_name(hardware)
    }
}

pub(super) struct ReceiverTable(BTreeMap<(u16, u16), String>);

impl ReceiverTable {
    fn new(rows: Vec<Receiver>) -> Result<Self, &'static str> {
        let mut receivers = BTreeMap::new();
        for receiver in rows {
            let vid = usb_id(&receiver.vid)
                .filter(|id| *id != 0)
                .ok_or("接收器 VID 无效")?;
            let pid = usb_id(&receiver.pid).ok_or("接收器 PID 无效")?;
            if !valid_text(&receiver.name, 64) {
                return Err("接收器名称无效");
            }
            if receivers.insert((vid, pid), receiver.name).is_some() {
                return Err("兼容包有重复的接收器 VID/PID");
            }
        }
        Ok(Self(receivers))
    }

    pub(super) fn receiver_name(&self, hardware: &str) -> Option<&str> {
        let (vid, pid) = hardware.strip_prefix("VID_")?.split_once("&PID_")?;
        self.0
            .get(&(usb_id(vid)?, usb_id(pid)?))
            .map(String::as_str)
    }
}

#[cfg(windows)]
pub(super) fn builtin_receivers() -> &'static ReceiverTable {
    static TABLE: std::sync::OnceLock<ReceiverTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let rows = serde_json::from_slice(BUILTIN).expect("内置接收器识别表应使用有效的 JSON");
        ReceiverTable::new(rows).expect("内置接收器识别表应包含有效的设备数据")
    })
}

fn usb_id(hex: &str) -> Option<u16> {
    if hex.len() != 4 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

fn valid_text(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.trim() == text
        && text.chars().count() <= max
        && !text.chars().any(char::is_control)
}

/// 供 USB 枚举线程读取当前启用的插件数据。安装状态由集市保存。
#[derive(Clone, Default)]
pub struct Store(Arc<Mutex<Option<Arc<Package>>>>);

impl Store {
    #[cfg(any(test, all(windows, feature = "sdk-js")))]
    pub(crate) fn set(&self, package: Option<Package>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = package.map(Arc::new);
    }

    pub(crate) fn package(&self) -> Option<Arc<Package>> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[cfg(all(windows, test))]
pub(super) fn builtin_data() -> Vec<serde_json::Value> {
    serde_json::from_slice(BUILTIN).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_receiver_extension_data() {
        let valid = json!({
            "id": PACKAGE_ID,
            "api_version": API_VERSION,
            "version": "1.1.0",
            "receivers": [{"vid": "046D", "pid": "c542", "name": "罗技 Nano 接收器"}]
        });
        let package = Package::parse(&serde_json::to_vec(&valid).unwrap()).unwrap();
        assert_eq!(package.version(), "1.1.0");
        assert_eq!(
            package.receiver_name("VID_046D&PID_C542"),
            Some("罗技 Nano 接收器")
        );
        assert_eq!(package.receiver_name("VID_046D&PID_C542&MI_00"), None);

        for (field, value) in [
            ("id", json!("example.receiver")),
            ("api_version", json!(2)),
            ("version", json!(" ")),
            ("cover_png", json!("封面由插件包提供")),
            (
                "receivers",
                json!([{"vid": "0000", "pid": "0001", "name": "接收器"}]),
            ),
            (
                "receivers",
                json!([{"vid": "046D", "pid": "C54", "name": "接收器"}]),
            ),
            (
                "receivers",
                json!([{"vid": "046D", "pid": "C542", "name": " "}]),
            ),
            (
                "receivers",
                json!([
                    {"vid": "046D", "pid": "C542", "name": "接收器"},
                    {"vid": "046d", "pid": "c542", "name": "重复接收器"}
                ]),
            ),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(
                Package::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{field}"
            );
        }
    }
}
