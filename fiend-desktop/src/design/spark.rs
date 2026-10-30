use gpui_kit::prelude::*;
use gpui_kit::{Bounds, IntoElement, PathBuilder, Pixels, Rgba, canvas, point, px};

pub const SPARK_HEIGHT: f32 = 26.;
const STROKE: f32 = 1.5;
const FILL_ALPHA: f32 = 0.12;

pub fn segments(
    values: &[Option<f32>],
    slots: usize,
    floor: f32,
    width: f32,
    height: f32,
) -> Vec<Vec<(f32, f32)>> {
    let slots = slots.max(2);
    let values = &values[values.len().saturating_sub(slots)..];
    let top = values.iter().flatten().copied().fold(floor, f32::max);
    if top <= 0. {
        return Vec::new();
    }
    let step = width / (slots - 1) as f32;
    let first = (slots - values.len()) as f32;
    let span = height - STROKE;
    let mut lines = Vec::new();
    let mut line = Vec::new();
    for (index, value) in values.iter().enumerate() {
        match value {
            Some(value) => {
                let x = (first + index as f32) * step;
                let y = STROKE / 2. + span * (1. - (value.max(0.) / top).min(1.));
                line.push((x, y));
            }
            None if !line.is_empty() => lines.push(std::mem::take(&mut line)),
            None => {}
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn sparkline(
    values: Vec<Option<f32>>,
    slots: usize,
    floor: f32,
    color: Rgba,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let at = |(x, y): (f32, f32)| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
            let mut fill = color;
            fill.a *= FILL_ALPHA;
            for line in segments(&values, slots, floor, width, height) {
                let (first, last) = (line[0], line[line.len() - 1]);
                let mut area = PathBuilder::fill();
                area.move_to(at((first.0, height)));
                for &dot in &line {
                    area.line_to(at(dot));
                }
                area.line_to(at((last.0, height)));
                area.close();
                if let Ok(path) = area.build() {
                    window.paint_path(path, fill);
                }
                let mut stroke = PathBuilder::stroke(px(STROKE));
                stroke.move_to(at(first));
                if line.len() == 1 {
                    stroke.line_to(at((first.0 + 0.5, first.1)));
                }
                for &dot in &line[1..] {
                    stroke.line_to(at(dot));
                }
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .w_full()
    .h(px(SPARK_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_samples_sit_at_the_right_edge() {
        let lines = segments(&[Some(5.), Some(10.)], 5, 1., 40., 21.5);
        assert_eq!(lines, vec![vec![(30., 10.75), (40., 0.75)]]);
    }

    #[test]
    fn gaps_split_the_line_and_the_floor_limits_the_scale() {
        let lines = segments(&[Some(1.), None, Some(2.), Some(2.)], 4, 4., 30., 11.5);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], vec![(0., 8.25)]);
        assert_eq!(lines[1], vec![(20., 5.75), (30., 5.75)]);
    }

    #[test]
    fn old_samples_drop_off_the_left_edge() {
        let values: Vec<_> = (0..10).map(|value| Some(value as f32)).collect();
        let lines = segments(&values, 3, 0., 20., 10.);
        assert_eq!(lines[0].len(), 3);
        assert_eq!(lines[0][0].0, 0.);
    }

    #[test]
    fn an_idle_line_stays_on_the_floor() {
        assert!(segments(&[Some(0.)], 4, 0., 10., 10.).is_empty());
        let lines = segments(&[Some(0.), Some(0.)], 4, 1., 30., 11.5);
        assert_eq!(lines[0][0].1, 10.75);
    }
}
