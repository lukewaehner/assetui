use ratatui::layout::Rect;

#[allow(dead_code)]
pub(crate) fn squarify(weights: &[f64], area: Rect) -> Vec<Rect> {
    let n = weights.len();
    let mut out = vec![Rect::new(area.x, area.y, 0, 0); n];
    let total: f64 = weights.iter().map(|w| w.max(0.0)).sum();
    if n == 0 || total <= 0.0 || area.width == 0 || area.height == 0 {
        return out;
    }

    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);

    let area_cells = f64::from(area.width) * f64::from(area.height);
    let areas: Vec<f64> = weights
        .iter()
        .map(|w| w.max(0.0) / total * area_cells)
        .collect();

    let mut fx = f64::from(area.x);
    let mut fy = f64::from(area.y);
    let mut fw = f64::from(area.width);
    let mut fh = f64::from(area.height);

    let mut i = 0;
    while i < n {
        let short = fw.min(fh);

        let mut j = i;
        let mut row_sum = 0.0;
        let mut best = f64::INFINITY;
        while j < n {
            let candidate = row_sum + areas[j];
            let worst = worst_ratio(&areas[i..=j], candidate, short);
            if worst > best {
                break;
            }
            best = worst;
            row_sum = candidate;
            j += 1;
        }
        if j == i {
            row_sum = areas[i];
            j = i + 1;
        }

        let thickness = if short > 0.0 { row_sum / short } else { 0.0 };
        let vertical_strip = fw >= fh;

        let mut pos = if vertical_strip { fy } else { fx };
        for k in i..j {
            let cell = if row_sum > 0.0 { areas[k] / row_sum * short } else { 0.0 };
            let mut r = if vertical_strip {
                Rect::new(
                    fx.round() as u16,
                    pos.round() as u16,
                    thickness.round() as u16,
                    cell.round() as u16,
                )
            } else {
                Rect::new(
                    pos.round() as u16,
                    fy.round() as u16,
                    cell.round() as u16,
                    thickness.round() as u16,
                )
            };
            r.x = r.x.min(max_x);
            r.y = r.y.min(max_y);
            r.width = r.width.min(max_x - r.x);
            r.height = r.height.min(max_y - r.y);
            out[k] = r;
            pos += cell;
        }

        if vertical_strip {
            fx += thickness;
            fw -= thickness;
        } else {
            fy += thickness;
            fh -= thickness;
        }
        i = j;
    }
    out
}

fn worst_ratio(areas: &[f64], row_sum: f64, short: f64) -> f64 {
    if row_sum <= 0.0 || short <= 0.0 {
        return f64::INFINITY;
    }
    let max = areas.iter().copied().fold(f64::MIN, f64::max);
    let min = areas.iter().copied().fold(f64::MAX, f64::min);
    let s2 = short * short;
    let sum2 = row_sum * row_sum;
    (s2 * max / sum2).max(sum2 / (s2 * min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(area: Rect, r: Rect) -> bool {
        r.x >= area.x
            && r.y >= area.y
            && r.x + r.width <= area.x + area.width
            && r.y + r.height <= area.y + area.height
    }

    #[test]
    fn squarify_empty_input_is_empty() {
        let area = Rect::new(0, 0, 40, 20);
        assert!(squarify(&[], area).is_empty());
    }

    #[test]
    fn squarify_single_weight_fills_the_area() {
        let area = Rect::new(2, 3, 40, 20);
        let rects = squarify(&[1.0], area);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], area);
    }

    #[test]
    fn squarify_keeps_every_rect_within_the_area() {
        let cases: &[(&[f64], Rect)] = &[
            (&[5.0, 3.0, 2.0, 1.0, 1.0], Rect::new(0, 0, 60, 30)),
            (&[5.34, 17.39, 11.48, 11.48, 18.2], Rect::new(3, 2, 13, 30)),
            (&[1.0, 1.0, 1.0], Rect::new(5, 5, 7, 3)),
            (&[100.0, 1.0, 1.0, 1.0, 1.0, 1.0], Rect::new(0, 0, 41, 19)),
        ];
        for (weights, area) in cases {
            let rects = squarify(weights, *area);
            assert_eq!(rects.len(), weights.len());
            for r in &rects {
                assert!(contains(*area, *r), "rect {r:?} escaped {area:?} for {weights:?}");
            }
        }
    }

    #[test]
    fn squarify_bigger_weight_gets_more_area() {
        let area = Rect::new(0, 0, 60, 30);
        let rects = squarify(&[9.0, 1.0], area);
        let a0 = u32::from(rects[0].width) * u32::from(rects[0].height);
        let a1 = u32::from(rects[1].width) * u32::from(rects[1].height);
        assert!(a0 > a1, "first weight (9) should get more area than second (1)");
    }

    #[test]
    fn squarify_zero_weight_gets_zero_area() {
        let area = Rect::new(0, 0, 40, 20);
        let rects = squarify(&[1.0, 0.0], area);
        assert_eq!(u32::from(rects[1].width) * u32::from(rects[1].height), 0);
    }

    #[test]
    fn squarify_all_zero_or_empty_area_is_all_zero_area() {
        let area = Rect::new(0, 0, 40, 20);
        for r in squarify(&[0.0, 0.0], area) {
            assert_eq!(u32::from(r.width) * u32::from(r.height), 0);
        }
        let zero_area = Rect::new(0, 0, 0, 0);
        assert_eq!(squarify(&[1.0, 1.0], zero_area).len(), 2);
    }
}
