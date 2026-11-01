//! Finding the repos under a folder and measuring each: its lines of
//! code, its main language, and when it began and last changed. The
//! numbers are kept in ~/.galaxy/cache.tsv by commit, so a repo is only
//! measured again after its HEAD moves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Repo {
    pub name: String,
    pub path: PathBuf,
    /// The commit HEAD points at, as read from .git.
    pub head: String,
    pub lines: u64,
    pub lang: &'static str,
    /// Unix times of the first commit and of HEAD.
    pub born: i64,
    pub last: i64,
}

/// The languages told apart, and the extensions that count for each.
const LANGS: &[(&str, &[&str])] = &[
    ("Rust", &["rs"]),
    ("Assembly", &["asm", "s", "nasm", "inc"]),
    ("Ruby", &["rb", "gemspec", "rake"]),
    ("Python", &["py"]),
    ("Kotlin", &["kt", "kts", "java"]),
    ("C", &["c", "h", "cc", "cpp", "hpp"]),
    ("Shell", &["sh", "bash", "zsh"]),
    ("JavaScript", &["js", "mjs", "ts", "tsx", "jsx"]),
    ("Web", &["html", "css", "scss", "erb"]),
    ("Go", &["go"]),
    ("Lua", &["lua"]),
    ("HyperList", &["hl"]),
    ("LaTeX", &["tex"]),
    ("LilyPond", &["ly"]),
    ("HP calculator", &["41", "25", "29"]),
    ("Vim", &["vim"]),
    ("Text", &["md", "txt"]),
];

/// Folders that hold builds, downloads or other people's code.
const SKIP: &[&str] = &["target", "node_modules", "vendor", "dist", "build", "__pycache__",
                        "venv", ".venv", "pkg", "out", "gradle"];

/// A file bigger than this is data, not code someone wrote.
const MAX_FILE: u64 = 1 << 20;

fn lang_of(ext: &str) -> Option<&'static str> {
    LANGS.iter().find(|(_, exts)| exts.contains(&ext)).map(|(l, _)| *l)
}

/// A script without an extension names its language on the first line.
fn lang_of_script(p: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let n = std::fs::File::open(p).ok()?.read(&mut head).ok()?;
    let first = std::str::from_utf8(&head[..n]).ok()?.lines().next()?;
    if !first.starts_with("#!") {
        return None;
    }
    ["ruby", "python", "bash", "sh", "zsh"].iter()
        .find(|w| first.split(['/', ' ']).any(|p| p.starts_with(*w)))
        .map(|w| match *w {
            "ruby" => "Ruby",
            "python" => "Python",
            _ => "Shell",
        })
}

/// Lines per language in the tree under `dir`, skipping dot folders,
/// build output and big files.
fn count(dir: &Path, per: &mut HashMap<&'static str, u64>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if !SKIP.contains(&name.as_ref()) {
                count(&p, per);
            }
            continue;
        }
        if !ft.is_file() || e.metadata().map_or(true, |m| m.len() > MAX_FILE) {
            continue;
        }
        let lang = match name.rsplit_once('.') {
            Some((_, ext)) => lang_of(&ext.to_ascii_lowercase()),
            None => lang_of_script(&p),
        };
        if let Some(l) = lang {
            if let Ok(bytes) = std::fs::read(&p) {
                *per.entry(l).or_default() += bytes.iter().filter(|b| **b == b'\n').count() as u64;
            }
        }
    }
}

/// The repo's language is the one with the most lines; plain text only
/// when nothing else is there.
fn measure(dir: &Path) -> (u64, &'static str) {
    let mut per = HashMap::new();
    count(dir, &mut per);
    let text = per.remove("Text").unwrap_or(0);
    match per.iter().max_by_key(|(_, n)| **n) {
        Some((l, _)) => (per.values().sum(), l),
        None => (text, "Text"),
    }
}

/// The commit HEAD points at: a hash, or a branch looked up in refs and
/// then packed-refs. Two small reads, no git process.
pub fn head_of(repo: &Path) -> Option<String> {
    let git = repo.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else { return Some(head.to_string()) };
    if let Ok(h) = std::fs::read_to_string(git.join(r)) {
        return Some(h.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find(|l| l.ends_with(r)).and_then(|l| l.split(' ').next()).map(String::from)
}

fn git_times(repo: &Path) -> (i64, i64) {
    let run = |args: &[&str]| -> Vec<i64> {
        std::process::Command::new("git").arg("-C").arg(repo).args(args)
            .stderr(std::process::Stdio::null()).output().ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect())
            .unwrap_or_default()
    };
    let last = run(&["log", "-1", "--format=%ct"]).first().copied().unwrap_or(0);
    let born = run(&["log", "--max-parents=0", "--format=%ct", "HEAD"]).into_iter().min().unwrap_or(last);
    (born, last)
}

/// Measure one repo afresh.
pub fn measure_repo(path: &Path, head: String) -> Repo {
    let (lines, lang) = measure(path);
    let (born, last) = git_times(path);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Repo { name, path: path.to_path_buf(), head, lines, lang, born, last }
}

fn cache_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".galaxy").join("cache.tsv")
}

fn intern(lang: &str) -> &'static str {
    LANGS.iter().map(|(l, _)| *l).find(|l| *l == lang).unwrap_or("Text")
}

fn load_cache() -> HashMap<PathBuf, Repo> {
    let mut out = HashMap::new();
    let Ok(s) = std::fs::read_to_string(cache_path()) else { return out };
    for l in s.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() != 6 {
            continue;
        }
        let path = PathBuf::from(f[0]);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let (Ok(lines), Ok(born), Ok(last)) = (f[2].parse(), f[4].parse(), f[5].parse()) else { continue };
        out.insert(path.clone(), Repo { name, path, head: f[1].into(), lines, lang: intern(f[3]), born, last });
    }
    out
}

pub fn save_cache(repos: &[Repo]) {
    let mut s = String::new();
    for r in repos {
        s.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\n",
            r.path.display(), r.head, r.lines, r.lang, r.born, r.last));
    }
    let p = cache_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

/// Every repo directly under `root`, oldest first. Repos whose HEAD
/// moved since the cache was written are measured again, on all cores;
/// `progress` hears how many are left to measure.
pub fn scan(root: &Path, progress: &mut dyn FnMut(usize)) -> Vec<Repo> {
    let cache = load_cache();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root).map(|rd| {
        rd.flatten().map(|e| e.path()).filter(|p| p.join(".git").is_dir()).collect()
    }).unwrap_or_default();
    // Real folders before links, so a link to a repo already here is the
    // one left out; a folder of links alone works too.
    dirs.sort_by_key(|p| (p.is_symlink(), p.clone()));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    dirs.sort();
    let mut repos = Vec::with_capacity(dirs.len());
    let mut todo = Vec::new();
    for d in dirs {
        let head = head_of(&d).unwrap_or_default();
        match cache.get(&d) {
            Some(r) if r.head == head && !head.is_empty() => repos.push(r.clone()),
            _ => todo.push((d, head)),
        }
    }
    if !todo.is_empty() {
        progress(todo.len());
        let n = std::thread::available_parallelism().map_or(4, |n| n.get());
        let chunk = todo.len().div_ceil(n);
        let fresh: Vec<Repo> = std::thread::scope(|s| {
            let hs: Vec<_> = todo.chunks(chunk).map(|c| s.spawn(move || {
                c.iter().map(|(d, h)| measure_repo(d, h.clone())).collect::<Vec<_>>()
            })).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        repos.extend(fresh);
        save_cache(&repos);
    }
    repos.retain(|r| r.lines > 0);
    repos.sort_by_key(|r| (r.born, r.name.clone()));
    repos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_scripts_name_their_language() {
        assert_eq!(lang_of("rs"), Some("Rust"));
        assert_eq!(lang_of("asm"), Some("Assembly"));
        assert_eq!(lang_of("png"), None);
        let d = std::env::temp_dir().join(format!("galaxy-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::create_dir_all(d.join("target")).unwrap();
        std::fs::write(d.join("bin/tool"), "#!/usr/bin/env ruby\nputs 1\nputs 2\n").unwrap();
        std::fs::write(d.join("a.py"), "x = 1\n").unwrap();
        std::fs::write(d.join("target/big.rs"), "fn main() {}\n".repeat(50)).unwrap();
        let (lines, lang) = measure(&d);
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!((lines, lang), (4, "Ruby"));
    }
}
