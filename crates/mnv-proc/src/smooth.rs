//! 平滑：移动平均与 Savitzky-Golay。
//!
//! SG 系数通过最小二乘正规方程在局部窗口内求解（Savitzky & Golay, *Anal. Chem.*
//! **1964**, 36, 1627），不查表，因此窗口与阶数可任意组合。

use mnv_core::SmoothKind;

pub fn compute(y: &[f32], kind: SmoothKind, window: usize) -> Vec<f32> {
    match kind {
        SmoothKind::None => y.to_vec(),
        SmoothKind::MovingAverage => moving_average(y, window),
        SmoothKind::SavitzkyGolay => savitzky_golay(y, window, 3),
    }
}

pub fn moving_average(y: &[f32], window: usize) -> Vec<f32> {
    let n = y.len();
    let w = window.max(3) | 1;
    if n < 3 || w <= 1 {
        return y.to_vec();
    }
    let half = w / 2;
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + half + 1).min(n);
            let s: f32 = y[a..b].iter().sum();
            s / (b - a) as f32
        })
        .collect()
}

/// Savitzky-Golay 平滑（窗口内 `order` 阶多项式最小二乘拟合的中心值）。
pub fn savitzky_golay(y: &[f32], window: usize, order: usize) -> Vec<f32> {
    let n = y.len();
    let w = window.max(5) | 1;
    let half = w / 2;
    if n < w || w < 3 {
        return y.to_vec();
    }
    let m = order.clamp(1, w - 1).min(5);
    let coef = sg_coefficients(half as isize, m);

    (0..n)
        .map(|i| {
            let mut acc = 0.0f32;
            for (k, c) in coef.iter().enumerate() {
                let off = k as isize - half as isize;
                let idx = (i as isize + off).clamp(0, n as isize - 1) as usize;
                acc += *c as f32 * y[idx];
            }
            acc
        })
        .collect()
}

/// 计算中心点的平滑系数：
/// 解 `(AᵀA) x = e₀`，则 `c_j = Σ_k offset_j^k · x_k`。
fn sg_coefficients(half: isize, order: usize) -> Vec<f64> {
    let m = order;
    let dim = m + 1;
    let mut ata = vec![vec![0.0f64; dim]; dim];
    for off in -half..=half {
        let x = off as f64;
        let mut basis = vec![0.0f64; dim];
        let mut pw = 1.0;
        for b in basis.iter_mut() {
            *b = pw;
            pw *= x;
        }
        for r in 0..dim {
            for c in 0..dim {
                ata[r][c] += basis[r] * basis[c];
            }
        }
    }
    let mut rhs = vec![0.0f64; dim];
    rhs[0] = 1.0;
    let x = gauss_solve(ata, rhs);

    (-half..=half)
        .map(|off| {
            let o = off as f64;
            let mut s = 0.0;
            let mut pw = 1.0;
            for xk in x.iter() {
                s += xk * pw;
                pw *= o;
            }
            s
        })
        .collect()
}

fn gauss_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for c in col..n {
            a[col][c] /= d;
        }
        b[col] /= d;
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r][col];
            if f.abs() < 1e-18 {
                continue;
            }
            for c in col..n {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sg_preserves_a_straight_line() {
        let y: Vec<f32> = (0..101).map(|i| 2.0 + 0.5 * i as f32).collect();
        let s = savitzky_golay(&y, 11, 3);
        for i in 20..80 {
            assert!((s[i] - y[i]).abs() < 1e-3, "i={i} {} vs {}", s[i], y[i]);
        }
    }

    #[test]
    fn sg_coefficients_sum_to_one() {
        let c = sg_coefficients(5, 3);
        let sum: f64 = c.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn moving_average_reduces_noise() {
        let y: Vec<f32> = (0..200)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let s = moving_average(&y, 9);
        assert!(s[100].abs() < 0.3);
    }
}
