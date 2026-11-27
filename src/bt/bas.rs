//! 单个标准电池服务实例的电量与供电状态（BAS 1.1）。
#[derive(Debug, PartialEq)]
pub(super) struct LevelStatus {
    present: bool,
    level: Option<u8>,
    charging: Option<bool>,
}

pub(super) fn parse_status(bytes: &[u8]) -> Option<LevelStatus> {
    let [flags, low, high, rest @ ..] = bytes else { return None };
    let offset = if flags & 1 != 0 { 2 } else { 0 };
    let has_level = flags & 2 != 0;
    let length = offset + usize::from(has_level) + usize::from(flags & 4 != 0);
    if rest.len() < length { return None; }
    let power = u16::from_le_bytes([*low, *high]);
    let charging = match (power >> 5) & 3 {
        1 => Some(true),
        2 | 3 => Some(false),
        _ => None,
    };
    let level = has_level.then(|| rest[offset]).filter(|&v| v <= 100);
    Some(LevelStatus { present: power & 1 != 0, level, charging })
}

pub(super) fn service_level(level: Option<u8>, status: Option<LevelStatus>) -> Option<(u8, Option<bool>)> {
    match status {
        Some(s) if s.present => Some((s.level.or(level)?, s.charging)),
        Some(_) => None,
        None => Some((level?, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_optional_percentage_after_identifier() {
        let report = [7, 0x21, 0, 0x34, 0x12, 42, 0];
        assert_eq!(service_level(None, parse_status(&report)), Some((42, Some(true))));
        for end in 0..report.len() { assert!(parse_status(&report[..end]).is_none()); }
    }

    #[test]
    fn distinguishes_empty_unknown_and_absent_battery() {
        assert_eq!(service_level(None, parse_status(&[2, 0x41, 0, 0])), Some((0, Some(false))));
        assert_eq!(service_level(Some(75), parse_status(&[2, 1, 0, 255])), Some((75, None)));
        assert_eq!(service_level(Some(0), parse_status(&[0, 0, 0])), None);
        assert_eq!(service_level(Some(18), parse_status(&[2, 0x21])), Some((18, None)));
    }
}
