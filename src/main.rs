//! galaxy: your repos as a spiral galaxy. A star for each repo, sized by
//! its lines of code and coloured by its language; the oldest by the
//! core, the newest at the arm tips. A commit makes its star flare.

mod scan;
mod sky;

use crust::{style, Crust, Cursor, Input, Pane};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Width of the panel on the right.
const INFO_W: u16 = 34;
/// How long a star flares after a commit.
const FLARE: Duration = Duration::from_millis(2500);
/// Labels for the biggest repos, besides the one picked.
const LABELS: usize = 14;

struct App {
    repos: Vec<scan::Repo>,
    /// Each repo's place on the unit disc, in the order of `repos`.
    pos: Vec<(f64, f64)>,
    sel: usize,
    flares: Vec<(usize, Instant)>,
    /// The galaxy's glow for one canvas size, drawn once.
    backdrop: Option<((usize, usize), Vec<u8>)>,
    /// The glow with the rings and stars on it, for a canvas size and a
    /// generation of the repos.
    layer: Option<((usize, usize, u64), Vec<u8>)>,
    /// Counts the commits seen, so the stars are painted again after one.
    generation: u64,
    /// The last canvas size, so arrows move the way the picture looks.
    canvas: (usize, usize),
    images: Option<glow::Display>,
    /// Small pictures over the big one, around the stars that flare, so a
    /// flare frame sends a few cells instead of the whole galaxy.
    patch: Option<glow::Display>,
    note: Option<String>,
    now: i64,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn ymd(t: i64) -> String {
    // SAFETY: localtime_r only writes the tm we hand it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as _;
    unsafe { libc::localtime_r(&tt, &mut tm) };
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

fn ago(secs: i64) -> String {
    match secs {
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s if s < 86400 * 60 => format!("{} days ago", s / 86400),
        s if s < 86400 * 730 => format!("{} months ago", s / (86400 * 30)),
        s => format!("{} years ago", s / (86400 * 365)),
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn short(n: u64) -> String {
    if n >= 1000 { format!("{}k", (n + 500) / 1000) } else { n.to_string() }
}

/// The folder to map: the argument, else `root = ...` in ~/.galaxyrc,
/// else the current folder.
fn root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let tilde = |s: &str| match s.strip_prefix("~/") {
        Some(rest) => PathBuf::from(&home).join(rest),
        None => PathBuf::from(s),
    };
    if let Some(a) = std::env::args().skip(1).find(|a| !a.starts_with('-')) {
        return tilde(&a);
    }
    let rc = std::fs::read_to_string(PathBuf::from(&home).join(".galaxyrc")).unwrap_or_default();
    for l in rc.lines() {
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == "root" {
                return tilde(v.trim());
            }
        }
    }
    PathBuf::from(".")
}

impl App {
    fn age_days(&self, i: usize) -> f64 {
        (self.now - self.repos[i].last) as f64 / 86400.0
    }

    /// The star nearest the picked one in the direction (dx, dy), as the
    /// picture shows them; the picked one if there is none that way.
    fn step(&self, dx: f64, dy: f64) -> usize {
        let (_, _, rx, ry) = sky::frame(self.canvas.0.max(1), self.canvas.1.max(1));
        let at = |i: usize| (self.pos[i].0 * rx, self.pos[i].1 * ry);
        let (sx, sy) = at(self.sel);
        let mut best = (f64::MAX, self.sel);
        for j in 0..self.repos.len() {
            let (x, y) = at(j);
            let (vx, vy) = (x - sx, y - sy);
            let along = vx * dx + vy * dy;
            let across = (vx * dy - vy * dx).abs();
            if j != self.sel && along > 0.0 && across < along * 1.5 {
                let score = along + 2.0 * across;
                if score < best.0 {
                    best = (score, j);
                }
            }
        }
        best.1
    }

    fn find(&self, text: &str) -> Option<usize> {
        let t = text.to_lowercase();
        let hits = |exact: bool| self.repos.iter().position(|r| {
            let n = r.name.to_lowercase();
            if exact { n == t } else { n.contains(&t) }
        });
        hits(true).or_else(|| hits(false))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("galaxy — your repos as a spiral galaxy (Fe2O3 suite)");
        println!();
        println!("Usage: galaxy [FOLDER]");
        println!();
        println!("  FOLDER   the folder your repos are in; else `root = ...` in");
        println!("           ~/.galaxyrc; else the current folder");
        println!();
        println!("Each repo is a star: its size from its lines of code, its colour from its");
        println!("language. The oldest sit by the core, the newest at the arm tips. A repo");
        println!("changed this week has spikes, and a commit while galaxy is open flares.");
        println!();
        println!("Keys: arrows  move    Tab / Shift-Tab  newer / older    /  find");
        println!("      Enter  open the repo in pointer    q  quit");
        return;
    }
    if args.iter().any(|a| a == "-v" || a == "--version") {
        println!("galaxy {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let root = root();
    let repos = scan::scan(&root, &mut |n| eprintln!("Measuring {n} repos..."));
    if repos.is_empty() {
        eprintln!("No git repos in {}. Give the folder they are in: galaxy ~/code", root.display());
        std::process::exit(1);
    }
    let now = unix_now();
    let sel = (0..repos.len()).max_by_key(|i| repos[*i].last).unwrap_or(0);
    let mut app = App {
        pos: sky::layout(&repos),
        repos,
        sel,
        flares: Vec::new(),
        backdrop: None,
        layer: None,
        generation: 0,
        canvas: (0, 0),
        images: None,
        patch: None,
        note: None,
        now,
    };
    let watch = watch::Watch::new(&app.repos);

    Crust::init();
    Crust::set_app_identity("Galaxy");
    Crust::clear_screen();
    // The first read sets up the terminal's event source, which also
    // catches a resize; without it a resize before any key goes unseen.
    while Input::getchr_ms(0).is_some() {}
    render(&mut app);
    loop {
        let flaring = !app.flares.is_empty();
        // Block until a key or a commit. Only a running flare wakes on a
        // timer, and only until it ends.
        let mut fds = [
            libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: watch.as_ref().map_or(-1, |w| w.fd()), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two pollfds that live across the call.
        unsafe { libc::poll(fds.as_mut_ptr(), 2, if flaring { 80 } else { -1 }) };
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            break;
        }
        let mut dirty = false;
        if fds[1].revents & libc::POLLIN != 0 {
            if let Some(w) = &watch {
                for i in w.changed() {
                    if commit_seen(&mut app, i) {
                        dirty = true;
                    }
                }
            }
        }
        let mut quit = false;
        // The terminal may hand over several keys at once: take them all.
        while let Some(key) = Input::getchr_ms(0) {
            app.note = None;
            let mut seen = true;
            match key.as_str() {
                "q" | "Q" => quit = true,
                "LEFT" => app.sel = app.step(-1.0, 0.0),
                "RIGHT" => app.sel = app.step(1.0, 0.0),
                "UP" => app.sel = app.step(0.0, -1.0),
                "DOWN" => app.sel = app.step(0.0, 1.0),
                "TAB" => app.sel = (app.sel + 1) % app.repos.len(),
                "S-TAB" => app.sel = (app.sel + app.repos.len() - 1) % app.repos.len(),
                "/" => {
                    let (cols, rows) = Crust::terminal_size();
                    let mut ask = Pane::new(1, rows, cols, 1, 255, 236);
                    let t = ask.ask("Find: ", "");
                    match app.find(t.trim()) {
                        Some(i) => app.sel = i,
                        None if !t.trim().is_empty() => app.note = Some(format!("No repo named like \"{}\"", t.trim())),
                        None => {}
                    }
                }
                "ENTER" => open_pointer(&mut app),
                "RESIZE" => {
                    Crust::clear_screen();
                }
                _ => seen = false,
            }
            dirty |= seen;
        }
        if quit {
            break;
        }
        app.flares.retain(|(_, t)| t.elapsed() < FLARE);
        // The frame after the last flare goes in full, to take it away.
        if dirty || (flaring && app.flares.is_empty()) {
            render(&mut app);
        } else if flaring {
            render_flares(&mut app);
        }
    }
    // A picture left placed would sit over the shell after we exit.
    hide_pictures(&mut app);
    Crust::cleanup();
}

/// Take the sky and any flare patch off the screen.
fn hide_pictures(app: &mut App) {
    if let Some(d) = app.images.as_mut() {
        d.clear_all();
    }
    if let Some(p) = app.patch.as_mut() {
        p.drop_canvas();
    }
}

/// A repo's reflog changed: if HEAD moved, measure it again and flare.
fn commit_seen(app: &mut App, i: usize) -> bool {
    let r = &app.repos[i];
    let head = scan::head_of(&r.path).unwrap_or_default();
    if head == r.head {
        return false;
    }
    let fresh = scan::measure_repo(&r.path.clone(), head);
    app.note = Some(format!("{}: a new commit", fresh.name));
    app.repos[i] = fresh;
    app.generation += 1;
    app.now = unix_now();
    scan::save_cache(&app.repos);
    app.flares.retain(|(j, _)| *j != i);
    app.flares.push((i, Instant::now()));
    true
}

fn open_pointer(app: &mut App) {
    hide_pictures(app);
    Crust::cleanup();
    Crust::clear_screen();
    let ok = std::process::Command::new("pointer").arg(&app.repos[app.sel].path).status().is_ok();
    Crust::init();
    Crust::set_app_identity("Galaxy");
    Crust::clear_screen();
    if !ok {
        app.note = Some("pointer is not on the PATH".into());
    }
}

type Label = (usize, usize, String, (u8, u8, u8), bool);

/// The picture's size in cells, for a terminal of cols × rows.
fn pic_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.saturating_sub(INFO_W + 1).max(10), rows.saturating_sub(2).max(4))
}

/// Paint everything: the galaxy, its labels, the panel and the bars.
fn render(app: &mut App) {
    let (cols, rows) = Crust::terminal_size();
    let (pic_cols, pic_rows) = pic_size(cols, rows);
    let (cv, labels, pixels) = scene(app, pic_cols, pic_rows);
    if pixels {
        let mut out = String::new();
        for (row, col, text, c, bold) in &labels {
            out.push_str(&Cursor::at(*col as u16 + 1, *row as u16 + 2));
            out.push_str(&style::rgb(text, Some(*c), None, if *bold { "b" } else { "" }));
        }
        print!("{out}");
        if let Some(d) = app.images.as_mut() {
            d.swap_canvas(&cv, 1, 2);
        }
        if let Some(p) = app.patch.as_mut() {
            p.drop_canvas();
        }
    } else {
        let mut pane = Pane::new(1, 2, pic_cols, pic_rows, 255, 0);
        pane.wrap = false;
        pane.scroll = false;
        pane.set_text(&half_blocks(&cv));
        pane.refresh();
    }
    render_panel(app, pic_cols + 2, pic_rows);
    render_bars(app, cols, rows);
}

/// A flare frame: the galaxy is painted again, but only the cells around
/// each flaring star go to the terminal.
fn render_flares(app: &mut App) {
    let (cols, rows) = Crust::terminal_size();
    let (pic_cols, pic_rows) = pic_size(cols, rows);
    let (cv, _, pixels) = scene(app, pic_cols, pic_rows);
    if !pixels {
        return render(app);
    }
    let (cw, ch) = (cv.cell_w(), cv.cell_h());
    let (cx, cy, rx, ry) = sky::frame(cv.w, cv.h);
    let mut patches = Vec::new();
    for (i, _) in &app.flares {
        let (x, y) = (cx + app.pos[*i].0 * rx, cy + app.pos[*i].1 * ry);
        let ext = sky::flare_reach(sky::radius(app.repos[*i].lines, cw));
        let c0 = ((x - ext) / cw).floor().max(0.0) as usize;
        let r0 = ((y - ext) / ch).floor().max(0.0) as usize;
        let c1 = (((x + ext) / cw).ceil() as usize).min(pic_cols as usize);
        let r1 = (((y + ext) / ch).ceil() as usize).min(pic_rows as usize);
        if c1 > c0 && r1 > r0 {
            patches.push((crop(&cv, c0, r0, c1, r1), c0 as u16 + 1, r0 as u16 + 2));
        }
    }
    let list: Vec<(&glow::Canvas, u16, u16)> = patches.iter().map(|(c, x, y)| (c, *x, *y)).collect();
    app.patch.get_or_insert_with(glow::Display::new).swap_canvases(&list);
}

/// The cells from (c0, r0) up to (c1, r1) of a canvas, as a canvas.
fn crop(cv: &glow::Canvas, c0: usize, r0: usize, c1: usize, r1: usize) -> glow::Canvas {
    let (cw, ch) = (cv.cell_w(), cv.cell_h());
    let x0 = (c0 as f64 * cw).round() as usize;
    let y0 = (r0 as f64 * ch).round() as usize;
    let x1 = ((c1 as f64 * cw).round() as usize).min(cv.w);
    let y1 = ((r1 as f64 * ch).round() as usize).min(cv.h);
    let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        rgba.extend_from_slice(&cv.rgba[(y * cv.w + x0) * 4..(y * cv.w + x1) * 4]);
    }
    glow::Canvas { cols: (c1 - c0) as u16, rows: (r1 - r0) as u16, cell: cv.cell, w: x1 - x0, h: y1 - y0, rgba }
}

/// The galaxy as pixels, with holes where its names go, and the names.
/// The flag says whether the terminal shows real pixels.
fn scene(app: &mut App, pic_cols: u16, pic_rows: u16) -> (glow::Canvas, Vec<Label>, bool) {
    let d = app.images.get_or_insert_with(glow::Display::new);
    let pixels = d.real_pixels();
    let mut cv = if pixels {
        glow::Canvas::new(pic_cols, pic_rows)
    } else {
        glow::Canvas::with_cell(pic_cols, pic_rows, (1, 2))
    };
    app.canvas = (cv.w, cv.h);
    let (cx, cy, rx, ry) = sky::frame(cv.w, cv.h);
    let at = |i: usize| (cx + app.pos[i].0 * rx, cy + app.pos[i].1 * ry);
    let cell_w = cv.cell_w();

    // Up to three rings where a year begins, newest first, kept apart so
    // their labels do not crowd.
    let n = app.repos.len();
    let mut rings: Vec<(f64, String)> = Vec::new();
    if pixels && n >= 8 {
        let mut last_t = 2.0;
        for i in (1..n).rev() {
            let year = &ymd(app.repos[i].born)[..4];
            if year == &ymd(app.repos[i - 1].born)[..4] {
                continue;
            }
            let t = i as f64 / (n - 1) as f64;
            if (0.1..0.92).contains(&t) && last_t - t >= 0.12 && rings.len() < 3 {
                rings.push((sky::rank_radius(t), year.to_string()));
                last_t = t;
            }
        }
    }

    // The backdrop, rings and stars change only with the size or a
    // commit, so they are painted once and copied for every key.
    let key = (cv.w, cv.h, app.generation);
    if app.layer.as_ref().map(|l| l.0) != Some(key) {
        if app.backdrop.as_ref().map(|b| b.0) != Some((cv.w, cv.h)) {
            app.backdrop = Some(((cv.w, cv.h), sky::backdrop(cv.w, cv.h)));
        }
        if let Some((_, px)) = &app.backdrop {
            cv.rgba.copy_from_slice(px);
        }
        for (rad, _) in &rings {
            sky::ring(&mut cv, *rad);
        }
        for i in 0..n {
            let r = &app.repos[i];
            let (x, y) = at(i);
            let m = sky::Mark { age_days: app.age_days(i), flare: None };
            sky::star(&mut cv, x, y, sky::radius(r.lines, cell_w), sky::colour(r.lang), m);
        }
        app.layer = Some((key, cv.rgba.clone()));
    } else if let Some((_, px)) = &app.layer {
        cv.rgba.copy_from_slice(px);
    }
    for (i, t) in &app.flares {
        let r = &app.repos[*i];
        let (x, y) = at(*i);
        let m = sky::Mark { age_days: app.age_days(*i), flare: Some(t.elapsed().as_secs_f64() / FLARE.as_secs_f64()) };
        sky::star(&mut cv, x, y, sky::radius(r.lines, cell_w), sky::colour(r.lang), m);
    }
    let (x, y) = at(app.sel);
    sky::pick(&mut cv, x, y, sky::radius(app.repos[app.sel].lines, cell_w));

    // Names: the picked repo first, then the biggest, where there is room.
    let mut labels: Vec<Label> = Vec::new();
    if pixels {
        let ch = cv.cell_h();
        let mut taken: Vec<(usize, usize, usize)> = Vec::new();
        let mut big: Vec<usize> = (0..n).filter(|i| *i != app.sel).collect();
        big.sort_by_key(|i| std::cmp::Reverse(app.repos[*i].lines));
        for (k, &i) in std::iter::once(&app.sel).chain(big.iter().take(LABELS)).enumerate() {
            let (x, y) = at(i);
            let rad = sky::radius(app.repos[i].lines, cell_w);
            let row = (y / ch) as usize;
            let col = ((x + rad * 2.6 + 4.0) / cell_w).ceil() as usize;
            let name = app.repos[i].name.clone();
            let len = name.chars().count();
            if row >= pic_rows as usize || col + len >= pic_cols as usize {
                continue;
            }
            if taken.iter().any(|(r, c, l)| *r == row && col < c + l + 1 && *c < col + len + 1) {
                continue;
            }
            taken.push((row, col, len));
            let c = sky::colour(app.repos[i].lang);
            let c = if k == 0 { (255, 200, 120) } else { (c.0 / 2 + 100, c.1 / 2 + 100, c.2 / 2 + 100) };
            labels.push((row, col, name, c, k == 0));
        }
        // A year label tries the ring's top, bottom, right and left.
        for (rad, year) in &rings {
            let spots = [(cx, cy - rad * ry), (cx, cy + rad * ry), (cx + rad * rx, cy), (cx - rad * rx, cy)];
            for (x, y) in spots {
                let (row, col) = ((y / ch) as usize, ((x / cell_w) as usize).saturating_sub(2));
                if row > 0 && row < pic_rows as usize
                    && !taken.iter().any(|(r, c, l)| *r == row && col < c + l + 1 && *c < col + 5) {
                    taken.push((row, col, 4));
                    labels.push((row, col, year.clone(), (140, 150, 180), false));
                    break;
                }
            }
        }
    }

    for (row, col, text, _, _) in &labels {
        cv.hole(*row, *col, text.chars().count());
    }
    (cv, labels, pixels)
}

/// The canvas as half-block cells: the top pixel the foreground of ▀,
/// the bottom one its background.
fn half_blocks(cv: &glow::Canvas) -> String {
    let px = |x: usize, y: usize| {
        let o = (y * cv.w + x) * 4;
        (cv.rgba[o], cv.rgba[o + 1], cv.rgba[o + 2])
    };
    let mut out = String::new();
    for row in 0..cv.h / 2 {
        let (mut fg, mut bg) = (None, None);
        for x in 0..cv.w {
            let (a, b) = (px(x, row * 2), px(x, row * 2 + 1));
            if bg != Some(b) {
                out.push_str(&style::set_bg_rgb(b.0, b.1, b.2));
                bg = Some(b);
            }
            if a == b {
                out.push(' ');
                continue;
            }
            if fg != Some(a) {
                out.push_str(&style::set_fg_rgb(a.0, a.1, a.2));
                fg = Some(a);
            }
            out.push('▀');
        }
        out.push_str(style::RESET);
        out.push('\n');
    }
    out
}

fn pill(name: &str, c: (u8, u8, u8), width: usize) -> String {
    let label = format!(" {name} ");
    let bar = width.saturating_sub(label.chars().count() + 2);
    format!("{}{}{}",
        style::rgb("▐", Some(c), None, ""),
        style::rgb(&label, Some((0, 0, 0)), Some(c), "b"),
        style::rgb(&format!("▌{}", "━".repeat(bar)), Some(c), None, ""))
}

fn tilde(p: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match p.strip_prefix(&home) {
        Ok(rest) if !home.is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

/// The panel: the picked repo, then a key to the colours.
fn render_panel(app: &App, x: u16, h: u16) {
    let w = INFO_W as usize;
    let r = &app.repos[app.sel];
    let c = sky::colour(r.lang);
    let mut out = pill(&r.name, c, w);
    out.push_str(&format!("\n {} · {} lines\n", style::rgb(r.lang, Some(c), None, "b"), thousands(r.lines)));
    out.push_str(&format!(" began    {}\n", ymd(r.born)));
    out.push_str(&format!(" changed  {}\n", ago(app.now - r.last)));
    out.push_str(&format!(" {}\n", style::dim(&tilde(&r.path))));
    out.push_str(&format!(" {} of {} by age\n\n", app.sel + 1, app.repos.len()));
    out.push_str(&pill("LANGUAGES", (153, 153, 204), w));
    out.push('\n');
    let mut per: Vec<(&str, usize, u64)> = Vec::new();
    for r in &app.repos {
        match per.iter_mut().find(|p| p.0 == r.lang) {
            Some(p) => {
                p.1 += 1;
                p.2 += r.lines;
            }
            None => per.push((r.lang, 1, r.lines)),
        }
    }
    per.sort_by_key(|p| std::cmp::Reverse((p.1, p.2)));
    for (lang, count, lines) in &per {
        out.push_str(&format!(" {} {:<15}{:>4} {:>6}\n",
            style::rgb("●", Some(sky::colour(lang)), None, ""), lang, count, short(*lines)));
    }
    out.push_str(&format!("\n{}\n", pill("ARMS", (153, 153, 204), w)));
    out.push_str(" Rust · machine code · Ruby · rest\n");
    out.push_str(&style::dim(" oldest by the core, newest out\n"));
    out.push_str(&style::dim(" spikes: changed this week"));
    let mut pane = Pane::new(x, 2, INFO_W, h, 252, 0);
    pane.wrap = false;
    pane.scroll = false;
    pane.set_text(&out);
    pane.refresh();
}

fn render_bars(app: &App, cols: u16, rows: u16) {
    let total: u64 = app.repos.iter().map(|r| r.lines).sum();
    let first = app.repos.first().map_or(0, |r| r.born);
    let head = format!(" {}  {} repos · {} lines · since {}",
        style::bold("galaxy"), app.repos.len(), thousands(total), &ymd(first)[..4]);
    let mut top = Pane::new(1, 1, cols, 1, 255, 236);
    top.wrap = false;
    top.scroll = false;
    top.set_text(&head);
    top.refresh();
    let keys = app.note.clone().unwrap_or_else(||
        " ←→↑↓ move · Tab newer · S-Tab older · / find · Enter pointer · q quit".into());
    let mut bottom = Pane::new(1, rows, cols, 1, 244, 236);
    bottom.wrap = false;
    bottom.scroll = false;
    bottom.set_text(&keys);
    bottom.refresh();
}

/// Watching each repo's reflog, which git writes on every commit, pull
/// and checkout. Nothing runs between those events.
#[cfg(target_os = "linux")]
mod watch {
    use std::collections::HashMap;
    use std::os::unix::ffi::OsStrExt;

    pub struct Watch {
        fd: i32,
        wds: HashMap<i32, usize>,
    }

    impl Watch {
        pub fn new(repos: &[crate::scan::Repo]) -> Option<Watch> {
            // SAFETY: plain syscalls on a descriptor this struct owns.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            if fd < 0 {
                return None;
            }
            let mut wds = HashMap::new();
            for (i, r) in repos.iter().enumerate() {
                let p = r.path.join(".git").join("logs").join("HEAD");
                let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { continue };
                let wd = unsafe { libc::inotify_add_watch(fd, c.as_ptr(), libc::IN_MODIFY) };
                if wd >= 0 {
                    wds.insert(wd, i);
                }
            }
            Some(Watch { fd, wds })
        }

        pub fn fd(&self) -> i32 {
            self.fd
        }

        /// The repos with news since the last call, each once.
        pub fn changed(&self) -> Vec<usize> {
            let mut buf = [0u8; 4096];
            let mut out = Vec::new();
            loop {
                let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                let mut o = 0;
                while o + 16 <= n as usize {
                    let wd = i32::from_ne_bytes(buf[o..o + 4].try_into().unwrap_or_default());
                    let len = u32::from_ne_bytes(buf[o + 12..o + 16].try_into().unwrap_or_default()) as usize;
                    if let Some(i) = self.wds.get(&wd) {
                        if !out.contains(i) {
                            out.push(*i);
                        }
                    }
                    o += 16 + len;
                }
            }
            out
        }
    }
}

/// Elsewhere there is no inotify, so no live flares.
#[cfg(not(target_os = "linux"))]
mod watch {
    pub struct Watch;
    impl Watch {
        pub fn new(_: &[crate::scan::Repo]) -> Option<Watch> { None }
        pub fn fd(&self) -> i32 { -1 }
        pub fn changed(&self) -> Vec<usize> { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_well() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
        assert_eq!(short(275923), "276k");
        assert_eq!(ago(3 * 86400), "3 days ago");
    }
}
