use super::COORD_LIMIT;
use svgtypes::{PathParser, PathSegment};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PathCommand {
    Move([f32; 2]),
    Line([f32; 2]),
    Quad([f32; 2], [f32; 2]),
    Cubic([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

pub(crate) fn parse(path: &str, budget: usize) -> Result<Vec<PathCommand>, String> {
    let mut current = [0.0, 0.0];
    let mut origin = current;
    let mut commands = Vec::new();
    for segment in PathParser::from(path) {
        if commands.len() == budget {
            return Err("画布路径合计最多 4000 条命令".into());
        }
        let point = |x, y, absolute| -> Result<[f32; 2], String> {
            Ok([
                coordinate(x, current[0], absolute)?,
                coordinate(y, current[1], absolute)?,
            ])
        };
        let command = match segment.map_err(|error| format!("画布路径无效（{error}）"))? {
            PathSegment::MoveTo { abs, x, y } => {
                current = point(x, y, abs)?;
                origin = current;
                PathCommand::Move(current)
            }
            PathSegment::LineTo { abs, x, y } => {
                current = point(x, y, abs)?;
                PathCommand::Line(current)
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                current[0] = coordinate(x, current[0], abs)?;
                PathCommand::Line(current)
            }
            PathSegment::VerticalLineTo { abs, y } => {
                current[1] = coordinate(y, current[1], abs)?;
                PathCommand::Line(current)
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let control = point(x1, y1, abs)?;
                current = point(x, y, abs)?;
                PathCommand::Quad(control, current)
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let first = point(x1, y1, abs)?;
                let second = point(x2, y2, abs)?;
                current = point(x, y, abs)?;
                PathCommand::Cubic(first, second, current)
            }
            PathSegment::ClosePath { .. } => {
                current = origin;
                PathCommand::Close
            }
            _ => return Err("画布路径只支持 M、L、H、V、Q、C、Z 命令".into()),
        };
        commands.push(command);
    }
    if commands.is_empty() {
        return Err("画布路径缺少命令".into());
    }
    Ok(commands)
}

fn coordinate(value: f64, current: f32, absolute: bool) -> Result<f32, String> {
    let resolved = if absolute {
        value
    } else {
        value + f64::from(current)
    };
    let limit = f64::from(COORD_LIMIT);
    if (-limit..=limit).contains(&value) && (-limit..=limit).contains(&resolved) {
        Ok(resolved as f32)
    } else {
        Err("画布路径坐标必须在 -2000 到 2000 之间".into())
    }
}
