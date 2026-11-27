//! 独立功能插件在设置侧栏与岛上快捷导航共用的静态矢量图标。
use crate::plugin_canvas::{parse_path, PathCommand};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "NavigationIconSpec")]
pub(crate) struct NavigationIcon {
    path: String,
    background: String,
    #[serde(skip)]
    commands: Vec<PathCommand>,
    #[serde(skip)]
    background_rgb: [u8; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigationIconSpec {
    path: String,
    background: String,
}

impl TryFrom<NavigationIconSpec> for NavigationIcon {
    type Error = &'static str;

    fn try_from(spec: NavigationIconSpec) -> Result<Self, Self::Error> {
        let NavigationIconSpec { path, background } = spec;
        let hex = background
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("navigationIcon.background 须为 #RRGGBB")?;
        let [_, red, green, blue] = u32::from_str_radix(hex, 16)
            .map_err(|_| "navigationIcon.background 须为 #RRGGBB")?
            .to_be_bytes();
        if path.len() > 1024 || !path.is_ascii() {
            return Err("navigationIcon.path 最多 1024 个 ASCII 字节");
        }
        let commands = parse_path(&path, 64)
            .map_err(|_| "navigationIcon.path 只支持 M、L、H、V、Q、C、Z，最多 64 条命令")?;
        let mut draws = false;
        let inside = |point: &[f32; 2]| point.iter().all(|value| (0.0..=24.0).contains(value));
        for command in &commands {
            let valid = match command {
                PathCommand::Move(to) => inside(to),
                PathCommand::Line(to) => inside(to),
                PathCommand::Quad(control, to) => inside(control) && inside(to),
                PathCommand::Cubic(first, second, to) => inside(first) && inside(second) && inside(to),
                PathCommand::Close => true,
            };
            if !valid {
                return Err("navigationIcon.path 坐标须在 0 到 24 之间");
            }
            draws |= !matches!(command, PathCommand::Move(_) | PathCommand::Close);
        }
        if !draws {
            return Err("navigationIcon.path 至少需要一条直线或曲线");
        }
        Ok(Self {
            path,
            background,
            commands,
            background_rgb: [red, green, blue],
        })
    }
}

impl NavigationIcon {
    pub(crate) fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    pub(crate) fn background(&self) -> [u8; 3] {
        self.background_rgb
    }
}
