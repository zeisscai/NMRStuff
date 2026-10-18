//! 基线校正。
//!
//! 四种实现均为公开算法：
//! * **Whittaker / asLS**：Eilers, P. H. C. *Anal. Chem.* **2003**, 75, 3631（非对称最小二乘平滑）；
//! * **SNIP**：Ryan, C. G. et al. *Nucl. Instrum. Methods Phys. Res. B* **1988**, 34, 396；
//! * **多项式拟合**：最小二乘正规方程（教科书方法）；
//! * **移动平均基线**：滚动最小值 + 平滑（形态学开运算的简化）。
//!
//! 返回值是**基线本身**，由调用方做减法。

use nmrstuff_core::BaselineKind;

pub fn compute(
    y: &[f32],
    kind: BaselineKind,
    lambda: f64,
    order: usize,
    iterations: usize,
) -> Vec<f32> {
    match kind {
        BaselineKind::None => vec![0.0; y.len()],
        BaselineKind::Whittaker => whittaker(y, lambda, 0.001, iterations.max(1)),
        BaselineKind::Snip => snip(y, iterations.max(1) * 10),
        BaselineKind::Polynomial => polynomial(y, order.max(1)),
        BaselineKind::MovingAverage => moving_average_baseline(y, order.max(3) * 8 + 1),
    }
}

/// 非对称 Whittaker 平滑（asLS）。
///
/// 目标：`min_z Σ w_i (y_i - z_i)² + λ Σ (Δ²z_i)²`，
/// 用共轭梯度求解 `(W + λDᵀD) z = W y`（矩阵从不显式构造）。
pub fn whittaker(y: &[f32], lambda: f64, asymmetric: f64, iterations: usize) -> Vec<f32> {
    let n = y.len();
    if n < 5 || !(lambda > 0.0) {
        return vec![0.0; n];
    }
    let yd: Vec<f64> = y.iter().map(|v| *v as f64).collect();
    let p = asymmetric.clamp(1e-4, 0.4999);
    let mut w = vec![1.0f64; n];
    let mut z = yd.clone();

    for _ in 0..iterations.max(1) {
        z = solve_weighted(&yd, &w, lambda);
        for i in 0..n {
            w[i] = if yd[i] > z[i] { p } else { 1.0 - p };
        }
    }
    z.iter().map(|v| *v as f32).collect()
}

/// SNIP 迭代削峰（LLS 变换域内取滑动最小值）。
pub fn snip(y: &[f32], iterations: usize) -> Vec<f32> {
    let n = y.len();
    if n < 5 {
        return vec![0.0; n];
    }
    // LLS 变换：z = ln(ln(sqrt(y+1)+1)+1)，使统计噪声近似常数方差
    let mut v: Vec<f64> = y
        .iter()
        .map(|a| {
            let a = (*a as f64).max(0.0);
            ((a + 1.0).sqrt() + 1.0).ln().max(1e-9).ln().max(0.0)
        })
        .collect();

    let m = iterations.max(1).min((n - 2).max(1));
    for p in (1..=m).rev() {
        let prev = v.clone();
        for i in 0..n {
            let a = prev[i.saturating_sub(p)];
            let b = prev[(i + p).min(n - 1)];
            let avg = 0.5 * (a + b);
            if avg < v[i] {
                v[i] = avg;
            }
        }
    }
    v.iter()
        .map(|z| {
            let t = z.exp() - 1.0;
            let u = t.max(0.0).exp() - 1.0;
            u.max(0.0).powi(2) as f32
        })
        .collect()
}

/// 低频多项式基线。
pub fn polynomial(y: &[f32], order: usize) -> Vec<f32> {
    let n = y.len();
    let m = order.clamp(1, 8).min(n.saturating_sub(1).max(1));
    if n < 3 {
        return vec![0.0; n];
    }
    let mut a = vec![vec![0.0f64; m + 1]; m + 1];
    let mut b = vec![0.0f64; m + 1];

    for i in 0..n {
        let x = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
        let mut basis = vec![0.0f64; m + 1];
        let mut pw = 1.0;
        for k in 0..=m {
            basis[k] = pw;
            pw *= x;
        }
        for r in 0..=m {
            b[r] += basis[r] * y[i] as f64;
            for c in 0..=m {
                a[r][c] += basis[r] * basis[c];
            }
        }
    }
    let coef = gauss_solve(a, b);

    (0..n)
        .map(|i| {
            let x = -1.0 + 2.0 * i as f64 / (n - 1) as f64;
            let mut v = 0.0;
            let mut pw = 1.0;
            for c in &coef {
                v += c * pw;
                pw *= x;
            }
            v.max(0.0) as f32
        })
        .collect()
}

/// 移动平均基线：滚动最小值 → 多次箱式平滑 → 与原始谱取小值。
pub fn moving_average_baseline(y: &[f32], window: usize) -> Vec<f32> {
    let n = y.len();
    if n < 5 {
        return vec![0.0; n];
    }
    let w = window.clamp(5, n / 2).max(5);
    let mut v = rolling_min(y, w);
    for _ in 0..3 {
        v = box_smooth(&v, w);
    }
    v.iter()
        .zip(y)
        .map(|(b, y)| (*b).min(*y).max(0.0))
        .collect()
}

fn rolling_min(y: &[f32], window: usize) -> Vec<f32> {
    let n = y.len();
    let half = window / 2;
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + half + 1).min(n);
            y[a..b].iter().copied().fold(f32::MAX, f32::min)
        })
        .collect()
}

fn box_smooth(y: &[f32], window: usize) -> Vec<f32> {
    let n = y.len();
    let half = window / 2;
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(half);
            let b = (i + half + 1).min(n);
            let s: f32 = y[a..b].iter().sum();
            s / (b - a) as f32
        })
        .collect()
}

/// 五对角对称正定系统的 LDLᵀ 分解（带宽 2），精确且 O(n)。
///
/// 求解 `(W + λDᵀD) z = W y` 时用它替代迭代法：`λ` 很大时矩阵条件数可达 1e6 量级，
/// 共轭梯度需要数百次迭代且精度不稳，而带状分解在这里只有 3 条对角线。
struct Band5 {
    main: Vec<f64>,
    off1: Vec<f64>,
    off2: Vec<f64>,
}

impl Band5 {
    fn build(w: &[f64], lambda: f64) -> Self {
        let n = w.len();
        let main: Vec<f64> = (0..n).map(|i| w[i] + lambda * 6.0).collect();
        let off1 = vec![-4.0 * lambda; n];
        let off2 = vec![lambda; n];
        Self { main, off1, off2 }
    }

    fn a(&self, i: usize, j: usize) -> f64 {
        debug_assert!(i >= j);
        match i - j {
            0 => self.main[i],
            1 => self.off1[j],
            2 => self.off2[j],
            _ => 0.0,
        }
    }

    fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = b.len();
        let mut d = vec![0.0f64; n];
        let mut l1 = vec![0.0f64; n];
        let mut l2 = vec![0.0f64; n];

        for i in 0..n {
            let k0 = i.saturating_sub(2);
            for j in k0..i {
                let mut s = self.a(i, j);
                for k in k0..j {
                    let ljk = if j - k == 2 { l2[j] } else { l1[j] };
                    let lik = if i - k == 2 { l2[i] } else { l1[i] };
                    s -= lik * d[k] * ljk;
                }
                let l = s / d[j].max(1e-300);
                if i - j == 1 {
                    l1[i] = l;
                } else {
                    l2[i] = l;
                }
            }
            let mut s = self.a(i, i);
            if i >= 1 {
                s -= l1[i] * l1[i] * d[i - 1];
            }
            if i >= 2 {
                s -= l2[i] * l2[i] * d[i - 2];
            }
            d[i] = s;
        }

        // 前代 L y = b
        let mut y = vec![0.0f64; n];
        for i in 0..n {
            let mut s = b[i];
            if i >= 1 {
                s -= l1[i] * y[i - 1];
            }
            if i >= 2 {
                s -= l2[i] * y[i - 2];
            }
            y[i] = s;
        }
        // 对角
        for i in 0..n {
            y[i] /= d[i].max(1e-300);
        }
        // 回代 Lᵀ x = z
        let mut x = y;
        for i in (0..n).rev() {
            if i + 1 < n {
                x[i] -= l1[i + 1] * x[i + 1];
            }
            if i + 2 < n {
                x[i] -= l2[i + 2] * x[i + 2];
            }
        }
        x
    }
}

fn solve_weighted(y: &[f64], w: &[f64], lambda: f64) -> Vec<f64> {
    let b: Vec<f64> = (0..y.len()).map(|i| w[i] * y[i]).collect();
    Band5::build(w, lambda).solve(&b)
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
        if a[piv][col].abs() < 1e-12 {
            continue;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
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

    fn noisy_baseline(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let x = i as f32 / n as f32;
                // 漂移基线 + 两个峰
                0.5 + 0.4 * x
                    + (-((x - 0.3) * 120.0).powi(2)).exp()
                    + 0.8 * (-((x - 0.7) * 160.0).powi(2)).exp()
            })
            .collect()
    }

    #[test]
    fn band_solver_matches_dense_system() {
        // 用显式矩阵验证五对角求解：A x = b，检查残差
        let n = 24usize;
        let lambda = 3.0;
        let w: Vec<f64> = (0..n).map(|i| if i % 5 == 0 { 0.01 } else { 1.0 }).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin() + 2.0).collect();
        let b: Vec<f64> = (0..n).map(|i| w[i] * y[i]).collect();
        let x = solve_weighted(&y, &w, lambda);

        // 手工构造 A x
        let mut ax = vec![0.0f64; n];
        for i in 0..n {
            let mut s = 0.0;
            for (off, c) in [(0isize, 6.0), (1, -4.0), (2, 1.0), (-1, -4.0), (-2, 1.0)] {
                let j = i as isize + off;
                if j >= 0 && (j as usize) < n {
                    s += c * x[j as usize];
                }
            }
            ax[i] = w[i] * x[i] + lambda * s;
        }
        let mut max_res = 0.0f64;
        for i in 0..n {
            max_res = max_res.max((ax[i] - b[i]).abs());
        }
        assert!(max_res < 1e-9, "残差 {max_res}");
    }

    #[test]
    fn whittaker_tracks_drift() {
        let n = 4096;
        let y = noisy_baseline(n);
        let b = whittaker(&y, 1e4, 0.001, 8);
        // 峰顶处基线应明显低于信号
        let peak = n * 3 / 10;
        assert!(b[peak] < y[peak] - 0.3, "基线 {} vs 信号 {}", b[peak], y[peak]);
        // 远离峰的平台上应贴近信号
        for i in [n / 20, n / 2 - n / 12, n - n / 20] {
            assert!(
                (b[i] - y[i]).abs() < 0.15,
                "i={i} 基线 {} vs 信号 {}",
                b[i],
                y[i]
            );
        }
    }

    #[test]
    fn snip_never_exceeds_signal() {
        let y = noisy_baseline(1024);
        let b = snip(&y, 60);
        for i in 0..y.len() {
            assert!(b[i] <= y[i] + 1e-4);
        }
    }

    #[test]
    fn polynomial_fits_linear_drift() {
        let y: Vec<f32> = (0..100).map(|i| 1.0 + 0.01 * i as f32).collect();
        let b = polynomial(&y, 2);
        assert!((b[50] - y[50]).abs() < 0.05);
    }
}
