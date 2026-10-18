//! 刻度生成：按 1 / 2 / 2.5 / 5 × 10ⁿ 取整步长。

/// 把原始步长向上取整到“好看”的数值。
pub fn next_nice_step(raw: f64) -> f64 {
    if !(raw > 0.0) || !raw.is_finite() {
        return 1.0;
    }
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let f = raw / base;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 2.5 {
        2.5
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

/// 在 `[lo, hi]` 上生成大约 `target` 个刻度。
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let (lo, hi) = if hi >= lo { (lo, hi) } else { (hi, lo) };
    let span = hi - lo;
    if !(span > 0.0) || !span.is_finite() {
        return vec![lo];
    }
    let step = next_nice_step(span / target.max(1) as f64);
    let first = (lo / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = first;
    // 防止浮点误差导致的无限循环
    for _ in 0..1000 {
        if v > hi + step * 1e-6 {
            break;
        }
        // 去掉 -0.0 与累积误差
        let snapped = (v / step).round() * step;
        out.push(if snapped.abs() < step * 1e-9 { 0.0 } else { snapped });
        v += step;
    }
    out
}

/// 按步长决定小数位，避免 “7.30 / 7.35” 混排。
pub fn tick_text(v: f64, step: f64) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        (-step.log10().floor()) as usize
    };
    let decimals = decimals + if (step / 10f64.powf(-(decimals as f64))).fract() > 1e-9 { 1 } else { 0 };
    format!("{:.*}", decimals.min(5), v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps() {
        assert!((next_nice_step(0.7) - 1.0).abs() < 1e-12);
        assert!((next_nice_step(1.3) - 2.0).abs() < 1e-12);
        assert!((next_nice_step(2.2) - 2.5).abs() < 1e-12);
        assert!((next_nice_step(4.1) - 5.0).abs() < 1e-12);
        assert!((next_nice_step(7.0) - 10.0).abs() < 1e-12);
        assert!((next_nice_step(0.13) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn ticks_cover_range() {
        let t = nice_ticks(0.0, 10.0, 5);
        assert!(t.len() >= 4);
        assert!((t[0] - 0.0).abs() < 1e-9);
        assert!(*t.last().unwrap() <= 10.0 + 1e-9);
    }

    #[test]
    fn ticks_descending_input_still_ascending_output() {
        let t = nice_ticks(12.0, 0.0, 6);
        for w in t.windows(2) {
            assert!(w[1] > w[0]);
        }
    }

    #[test]
    fn labels_have_consistent_decimals() {
        assert_eq!(tick_text(7.0, 1.0), "7");
        assert_eq!(tick_text(7.2, 0.2), "7.2");
        assert_eq!(tick_text(7.25, 0.05), "7.25");
    }
}
