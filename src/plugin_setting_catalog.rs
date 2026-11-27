use std::sync::OnceLock;

pub(crate) struct SettingIcon {
    pub name: &'static str,
    pub label: &'static str,
    pub path: &'static str,
}

pub(crate) const ICONS: &[SettingIcon] = &[
    SettingIcon { name: "heart", label: "爱心", path: "M12 21L4 13C0 9 2 4 7 4C9 4 11 5 12 7C13 5 15 4 17 4C22 4 24 9 20 13Z" },
    SettingIcon { name: "cake", label: "蛋糕", path: "M3 21V13H21V21Z M3 16Q6 19 9 16Q12 19 15 16Q18 19 21 16 M6 13V8 M12 13V8 M18 13V8 M6 5V4 M12 5V4 M18 5V4" },
    SettingIcon { name: "ring", label: "戒指", path: "M9 3H15L17 6L12 10L7 6Z M10 9C4 10 3 17 7 20C10 23 15 23 18 19C21 15 19 10 14 9" },
    SettingIcon { name: "plane", label: "飞机", path: "M22 2L15 22L11 13L2 9Z M22 2L11 13" },
    SettingIcon { name: "star", label: "星星", path: "M12 2L15 8L22 9L17 14L18 21L12 18L6 21L7 14L2 9L9 8Z" },
    SettingIcon { name: "gift", label: "礼物", path: "M4 12H20V21H4Z M2 8H22V12H2Z M12 8V21 M12 8H7C2 8 3 2 7 3C9 3 11 6 12 8C13 6 15 3 17 3C21 2 22 8 17 8Z" },
    SettingIcon { name: "flag", label: "旗子", path: "M5 22V3Q8 1 12 3Q16 5 20 3V14Q16 16 12 14Q8 12 5 14" },
    SettingIcon { name: "calendar", label: "日历", path: "M3 5H21V21H3Z M3 10H21 M8 3V7 M16 3V7 M8 15H10 M14 15H16" },
    SettingIcon { name: "coffee", label: "咖啡", path: "M3 9H17V14Q17 19 12 19H8Q3 19 3 14Z M17 10H19C24 10 22 16 17 16 M2 22H20 M7 3V6 M12 2V6" },
    SettingIcon { name: "coin", label: "硬币", path: "M22 12C22 17.5 17.5 22 12 22C6.5 22 2 17.5 2 12C2 6.5 6.5 2 12 2C17.5 2 22 6.5 22 12Z M8 7L12 12L16 7 M8 12H16 M8 15H16 M12 12V18" },
    SettingIcon { name: "sun", label: "太阳", path: "M17 12C17 15 15 17 12 17C9 17 7 15 7 12C7 9 9 7 12 7C15 7 17 9 17 12Z M12 2V4 M12 20V22 M2 12H4 M20 12H22 M5 5L6 6 M18 18L19 19 M5 19L6 18 M18 6L19 5" },
    SettingIcon { name: "moon", label: "月亮", path: "M20 15C11 18 6 9 12 3C5 3 2 8 3 14C4 22 16 24 20 15Z" },
];

pub(crate) fn icon_path(name: &str) -> Option<&'static str> {
    ICONS.iter().find(|icon| icon.name == name).map(|icon| icon.path)
}

pub(crate) struct SettingColor {
    pub name: &'static str,
    pub label: &'static str,
    pub rgb: [u8; 3],
}

pub(crate) const COLORS: &[SettingColor] = &[
    SettingColor { name: "red", label: "红", rgb: [226, 102, 99] },
    SettingColor { name: "orange", label: "橙", rgb: [239, 154, 79] },
    SettingColor { name: "yellow", label: "黄", rgb: [235, 194, 83] },
    SettingColor { name: "green", label: "绿", rgb: [121, 173, 103] },
    SettingColor { name: "teal", label: "青", rgb: [77, 173, 165] },
    SettingColor { name: "blue", label: "蓝", rgb: [99, 152, 229] },
    SettingColor { name: "purple", label: "紫", rgb: [160, 124, 212] },
    SettingColor { name: "pink", label: "粉", rgb: [216, 128, 166] },
];

pub(crate) fn setting_color(name: &str) -> Option<&'static SettingColor> {
    COLORS.iter().find(|color| color.name == name)
}

pub(crate) struct TimezoneCity {
    pub city: &'static str,
    pub region: &'static str,
    pub zone: &'static str,
}

pub(crate) fn timezone_cities(query: &str) -> impl Iterator<Item = &'static TimezoneCity> {
    static CITIES: OnceLock<Vec<TimezoneCity>> = OnceLock::new();
    let entries = CITIES.get_or_init(|| {
        include_str!("plugin_timezone_cities.tsv")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut columns = line.split('\t');
                Some(TimezoneCity {
                    city: columns.next()?,
                    region: columns.next()?,
                    zone: columns.next()?,
                })
            })
            .collect()
    });
    let query = query.trim().to_ascii_lowercase();
    entries.iter().filter(move |city| {
        city.city.contains(&query)
            || city.region.contains(&query)
            || city.zone.to_ascii_lowercase().contains(&query)
    })
}
