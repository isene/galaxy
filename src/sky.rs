//! Drawing the galaxy. Every repo sits on one of four spiral arms, by its
//! language: Rust, Ruby, machine code (assembly and calculator programs)
//! and the rest. The oldest repo sits by the core and the newest at the
//! tip of its arm, so the picture shows the work growing outward.

use crate::scan::Repo;
use glow::Canvas;

/// How tightly the arms wind: radians turned per e-fold of radius.
const WIND: f64 = 2.0;
/// Radius of the core, where the arms begin, as a share of the disc.
const R0: f64 = 0.14;

pub fn colour(lang: &str) -> (u8, u8, u8) {
    match lang {
        "Rust" => (255, 120, 40),
        "Assembly" => (120, 200, 255),
        "HP calculator" => (90, 230, 210),
        "Ruby" => (240, 60, 80),
        "Python" => (255, 215, 80),
        "Kotlin" => (180, 120, 255),
        "C" => (170, 180, 255),
        "Shell" => (120, 230, 120),
        "JavaScript" => (240, 230, 90),
        "Web" => (240, 130, 210),
        "Go" => (0, 190, 230),
        "Vim" => (100, 200, 100),
        "LaTeX" => (200, 200, 160),
        _ => (200, 200, 200),
    }
}

fn arm(lang: &str) -> usize {
    match lang {
        "Rust" => 0,
        "Ruby" => 2,
        "Assembly" | "HP calculator" => 1,
        _ => 3,
    }
}

fn arm_angle(k: usize) -> f64 {
    k as f64 * std::f64::consts::FRAC_PI_2 + 0.4
}

/// Where the arm through angle `base` passes at radius `r`.
fn spiral(base: f64, r: f64) -> f64 {
    base + (r.max(0.02) / R0).ln() * WIND
}

/// A number from 0 to 1, the same every time for the same input.
pub fn hash(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

fn name_hash(s: &str) -> u64 {
    s.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211))
}

/// The distance from the core for the repo at `t` of the way from the
/// oldest (0) to the newest (1).
pub fn rank_radius(t: f64) -> f64 {
    R0 + (0.95 - R0) * t.powf(0.85)
}

/// Each repo's place on the unit disc, in the order given (oldest first).
pub fn layout(repos: &[Repo]) -> Vec<(f64, f64)> {
    let n = repos.len().max(2) as f64;
    repos.iter().enumerate().map(|(i, r)| {
        let h = name_hash(&r.name);
        let t = i as f64 / (n - 1.0);
        let rad = rank_radius(t) + (hash(h) - 0.5) * 0.05;
        let th = spiral(arm_angle(arm(r.lang)), rad) + (hash(h ^ 7) - 0.5) * 0.5 / (1.0 + 3.0 * rad);
        (rad * th.cos(), rad * th.sin())
    }).collect()
}

/// The disc's centre and radii in pixels on a canvas of w by h.
pub fn frame(w: usize, h: usize) -> (f64, f64, f64, f64) {
    (w as f64 / 2.0, h as f64 / 2.0, w as f64 * 0.46, h as f64 * 0.45)
}

/// The glow of the galaxy without the repos: the core, four arms with
/// dust lanes, faint stars along them and in the field. Drawn once per
/// size, as RGBA. The light is smooth, so it is worked out on a grid of
/// every STEP-th pixel and blended between; the grain goes on per pixel.
pub fn backdrop(w: usize, h: usize) -> Vec<u8> {
    const STEP: usize = 4;
    let (cx, cy, rx, ry) = frame(w, h);
    let (gw, gh) = (w / STEP + 2, h / STEP + 2);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut grid = vec![[0f32; 6]; gw * gh];
    let per = gh.div_ceil(threads).max(1);
    bands(&mut grid, per * gw, |band, chunk| {
        for (i, g) in chunk.iter_mut().enumerate() {
            let (x, y) = ((i % gw) * STEP, (band * per + i / gw) * STEP);
            *g = light((x as f64 - cx) / rx, (y as f64 - cy) / ry);
        }
    });
    let mut px = vec![0u8; w * h * 4];
    let per = h.div_ceil(threads).max(1);
    let grid = &grid;
    bands(&mut px, per * w * 4, |band, chunk| {
        for (i, p) in chunk.chunks_mut(4).enumerate() {
            let (x, y) = (i % w, band * per + i / w);
            let (gx, gy) = (x / STEP, y / STEP);
            let (fx, fy) = ((x % STEP) as f32 / STEP as f32, (y % STEP) as f32 / STEP as f32);
            let (a, b) = (&grid[gy * gw + gx], &grid[gy * gw + gx + 1]);
            let (c, d) = (&grid[(gy + 1) * gw + gx], &grid[(gy + 1) * gw + gx + 1]);
            let grain = 0.7 + 0.6 * hash(x as u64 * 7919 + y as u64 * 104729) as f32;
            for k in 0..3 {
                let at = |j: usize| {
                    let top = a[j] + (b[j] - a[j]) * fx;
                    top + (c[j] + (d[j] - c[j]) * fx - top) * fy
                };
                p[k] = ((at(k) + at(k + 3) * grain) * 0.9).min(255.0) as u8;
            }
            p[3] = 255;
        }
    });
    let mut cv = Canvas { cols: 1, rows: 1, cell: (1, 1), w, h, rgba: px };
    sprinkle(&mut cv);
    cv.rgba
}

/// Runs `f` on each band of `data`, a thread per band. The web page has
/// one thread and no way to start another, so there the bands take turns.
fn bands<T: Send>(data: &mut [T], per: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if cfg!(target_os = "wasi") {
        data.chunks_mut(per).enumerate().for_each(|(band, chunk)| f(band, chunk));
        return;
    }
    let f = &f;
    std::thread::scope(|s| {
        for (band, chunk) in data.chunks_mut(per).enumerate() {
            s.spawn(move || f(band, chunk));
        }
    });
}

/// The light at (u, v) on the unit disc: the core's red, green and blue,
/// then the arms', kept apart so the grain can go on the arms alone.
fn light(u: f64, v: f64) -> [f32; 6] {
    let r = (u * u + v * v).sqrt();
    let th = v.atan2(u);
    let bulge = 0.75 * (-(r / 0.06).powi(2)).exp() + 0.22 * (-r / 0.16).exp();
    // Every arm winds the same way, so one logarithm serves all four.
    let wind = spiral(0.0, r);
    let width = 0.03 + 0.05 * r;
    let mut arms = 0.0;
    let mut lanes = 0.0;
    for k in 0..4 {
        let mut d = (th - wind - arm_angle(k)).rem_euclid(std::f64::consts::TAU);
        if d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        let z = d * r / width;
        if z.abs() < 4.0 {
            arms += (-z * z).exp();
            lanes += (-((d - 0.22) * r / (width * 0.45)).powi(2)).exp();
        }
    }
    let fade = ((1.08 - r) / 0.3).clamp(0.0, 1.0) * (r / 0.08).min(1.0);
    let disc = (arms * 0.26 * (-r / 0.6).exp() + 0.03 * (-r / 0.4).exp()) * fade * (1.0 - 0.6 * lanes.min(1.0));
    let warm = [255.0, 220.0, 170.0];
    let cool = [150.0, 170.0, 255.0];
    let mix = (r / 0.5).min(1.0);
    let mut out = [0f32; 6];
    for k in 0..3 {
        out[k] = (warm[k] * bulge) as f32;
        out[k + 3] = ((warm[k] + (cool[k] - warm[k]) * mix) * disc) as f32;
    }
    out
}

/// Faint stars: thousands along the arms, fewer across the whole field.
fn sprinkle(cv: &mut Canvas) {
    let (cx, cy, rx, ry) = frame(cv.w, cv.h);
    let area = (cv.w * cv.h) as f64;
    let n_arm = (area / 500.0) as u64;
    for i in 0..n_arm {
        let k = (hash(i * 3) * 4.0) as usize;
        let r = R0 * 0.6 + hash(i * 3 + 1).powf(0.7) * 0.95;
        let th = spiral(arm_angle(k), r) + (hash(i * 3 + 2) - 0.5) * 0.35 / (0.3 + r);
        let (x, y) = (cx + r * th.cos() * rx, cy + r * th.sin() * ry);
        let b = 0.25 + 0.5 * hash(i ^ 0xabcdef).powi(3);
        cv.blend(x as i64, y as i64, (220, 225, 255), b);
    }
    let n_field = (area / 1400.0) as u64;
    for i in 0..n_field {
        let (x, y) = (hash(i * 5 + 11) * cv.w as f64, hash(i * 5 + 12) * cv.h as f64);
        let b = 0.15 + 0.85 * hash(i * 5 + 13).powi(5);
        let tint = [(255, 240, 220), (220, 230, 255), (255, 210, 180)][(hash(i * 5 + 14) * 3.0) as usize % 3];
        cv.blend(x as i64, y as i64, tint, b);
        if b > 0.7 {
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                cv.blend(x as i64 + dx, y as i64 + dy, tint, b * 0.35);
            }
        }
    }
}

/// How a star is drawn beyond its size and colour.
#[derive(Clone, Copy, Default)]
pub struct Mark {
    /// Days since the last commit.
    pub age_days: f64,
    /// 0 to 1 through a flare after a commit seen live, or None.
    pub flare: Option<f64>,
}

/// A star's radius in pixels for `lines` of code, on a canvas whose
/// cells are `cell_w` pixels wide.
pub fn radius(lines: u64, cell_w: f64) -> f64 {
    (cell_w * 0.2 * (0.5 + 0.5 * (lines.max(10) as f64).log10())).max(0.6)
}

/// A flare is drawn at least this big, so a small repo's shows too.
fn flare_r(r: f64) -> f64 {
    r.max(2.5)
}

/// How far from its centre a star's flare paints, in pixels.
pub fn flare_reach(r: f64) -> f64 {
    flare_r(r) * 16.0 + 6.0
}

/// One repo's star at (x, y).
pub fn star(cv: &mut Canvas, x: f64, y: f64, r: f64, c: (u8, u8, u8), m: Mark) {
    let boost = m.flare.map_or(0.0, |f| 1.0 - f);
    let rf = flare_r(r);
    // A flare's glow: stacked discs, so it is brightest in the middle.
    for k in 1..=5 {
        if boost > 0.0 {
            cv.disc(x, y, rf * (3.0 + 9.0 * boost) * k as f64 / 5.0, c, 0.09 * boost * boost);
        }
    }
    cv.disc(x, y, r * 3.2, c, 0.10 + 0.2 * boost);
    cv.disc(x, y, r * 1.8, c, 0.28 + 0.3 * boost);
    cv.disc(x, y, r, c, 1.0);
    let hot = (((c.0 as u16 + 510) / 3) as u8, ((c.1 as u16 + 510) / 3) as u8, ((c.2 as u16 + 510) / 3) as u8);
    cv.disc(x, y, r * 0.5, hot, 0.9);
    // Spikes on a repo touched this week, longer the more recent.
    let recent = ((7.0 - m.age_days) / 7.0).clamp(0.0, 1.0);
    if recent > 0.0 || boost > 0.0 {
        let len = r * (2.0 + 4.0 * recent) + rf * 8.0 * boost;
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            cv.line((x + dx * r, y + dy * r), (x + dx * len, y + dy * len), 1.0, hot, 0.45 + 0.4 * boost);
        }
    }
    if let Some(f) = m.flare {
        cv.ring(x, y, rf * (2.0 + 12.0 * f), hot, 1.0 - f);
        cv.ring(x, y, rf * (2.0 + 12.0 * f) + 1.0, c, (1.0 - f) * 0.6);
    }
}

/// The orange double ring round the picked star.
pub fn pick(cv: &mut Canvas, x: f64, y: f64, r: f64) {
    let rr = r * 2.4 + 3.0;
    cv.ring(x, y, rr, (255, 153, 0), 1.0);
    cv.ring(x, y, rr + 1.0, (255, 153, 0), 1.0);
}

/// A faint dotted ellipse at `rad` of the disc, for an age ring.
pub fn ring(cv: &mut Canvas, rad: f64) {
    let (cx, cy, rx, ry) = frame(cv.w, cv.h);
    let steps = ((rx + ry) * rad * 2.0) as usize;
    for i in 0..steps {
        if i % 6 > 1 {
            continue;
        }
        let a = i as f64 / steps as f64 * std::f64::consts::TAU;
        cv.blend((cx + rad * rx * a.cos()) as i64, (cy + rad * ry * a.sin()) as i64, (120, 130, 160), 0.6);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, lang: &'static str) -> Repo {
        Repo { name: name.into(), path: name.into(), head: String::new(), lines: 1000, lang, born: 0, last: 0 }
    }

    #[test]
    fn oldest_sits_inside_newest() {
        let rs: Vec<Repo> = (0..20).map(|i| repo(&format!("r{i}"), "Rust")).collect();
        let p = layout(&rs);
        let rad = |q: (f64, f64)| (q.0 * q.0 + q.1 * q.1).sqrt();
        assert!(rad(p[0]) < 0.25 && rad(p[19]) > 0.85);
        assert!(p.iter().all(|q| rad(*q) < 1.0));
    }

    #[test]
    fn backdrop_has_a_bright_core_and_dark_corners() {
        let px = backdrop(200, 100);
        let at = |x: usize, y: usize| px[(y * 200 + x) * 4] as u32;
        assert!(at(100, 50) > 150);
        assert!(at(1, 1) < 60);
    }
}
