//! `cd` into an ODI file. `odi hook zsh` prints a small `cd` for the shell's
//! config: given a `.odi` file (or `app.odi/app`, or a hub name with a tag,
//! `acme/web:1.4`), it asks `odi open` for a folder that is the file, and goes
//! there. On Linux that's the file mounted read-only (FUSE), by an odi left
//! running in the background, read as it's looked at; elsewhere it's the
//! file unpacked once into a read-only folder. `odi close` lets them go.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use anyhow::anyhow;
use anyhow::{bail, Context, Result};

use crate::parts;

/// Where opened files are: ~/.odi/open/KEY.
fn base() -> PathBuf {
    crate::hub::odi_home().join("open")
}

/// `app.odi/app/lib` -> ("app.odi", "app/lib"); `acme/web:1.4/app` ->
/// ("acme/web:1.4", "app"); else (all of it, "").
fn split(arg: &str) -> (String, String) {
    let arg = arg.trim_end_matches('/');
    if let Some(i) = arg.find(".odi/") {
        return (arg[..i + 4].to_string(), arg[i + 5..].to_string());
    }
    if !Path::new(arg).exists() {
        if let Some(c) = arg.find(':') {
            if let Some(j) = arg[c..].find('/') {
                return (arg[..c + j].to_string(), arg[c + j + 1..].to_string());
            }
        }
    }
    (arg.to_string(), String::new())
}

/// The line for a shell's startup file: does nothing (and says nothing) once
/// odi isn't installed, so removing odi never breaks a terminal.
fn startup_line(shell: &str) -> String {
    match shell {
        "fish" => "command -q odi; and odi hook fish | source".into(),
        s => format!("command -v odi >/dev/null && eval \"$(odi hook {s})\""),
    }
}

pub fn cmd_hook(args: &[String]) -> Result<()> {
    let shell = match args {
        [s] => s.as_str(),
        _ => bail!(
            "usage: odi hook zsh|bash|fish, in your shell's startup file:\n  zsh (~/.zshrc):   {}\n  bash (~/.bashrc): {}\n  fish (config.fish): {}",
            startup_line("zsh"),
            startup_line("bash"),
            startup_line("fish")
        ),
    };
    match shell {
        "zsh" | "bash" | "sh" => print!("{POSIX_HOOK}"),
        "fish" => print!("{FISH_HOOK}"),
        other => bail!("no cd for {other:?}: zsh, bash or fish"),
    }
    Ok(())
}

/// The cd for zsh and bash (and sh). Given a .odi file, `app.odi/app`, or a
/// hub name with a tag (`acme/web:1.4`) that isn't a folder, it asks `odi
/// open` for the folder that is the file, and goes there. It remembers each
/// file's top and where `cd ..` from it goes (the folder the file is in, or
/// where cd was run for a hub name), so `..`, `../..` and `../x` inside an
/// opened file go up out of it. Anything else is the shell's own cd,
/// unchanged: options, `cd -`, no argument, two arguments, pushd and popd.
const POSIX_HOOK: &str = r#"# odi: cd into .odi files (cd app.odi), and hub names with a tag (cd acme/web:1.4)
__odi_back_of() { printf '%s\n' "${__ODI_TRAIL:-}" | awk -F'|' -v t="$1" '$1 == t { b = $2 } END { print b }'; }
__odi_inside() { printf '%s\n' "${__ODI_TRAIL:-}" | awk -F'|' -v p="$PWD" '$1 != "" && (p == $1 || index(p, $1 "/") == 1) { f = 1 } END { exit !f }'; }
cd() {
  # (.., ../.. and ../x inside an opened file: up past its top is the folder the file is in)
  if [ "$#" -eq 1 ] && [ -n "${__ODI_TRAIL:-}" ] && { [ "$1" = ".." ] || [ "${1#../}" != "$1" ]; } && __odi_inside; then
    __odi_cur=$PWD; __odi_rest=$1
    while [ "$__odi_rest" = ".." ] || [ "${__odi_rest#../}" != "$__odi_rest" ]; do
      __odi_b=$(__odi_back_of "$__odi_cur")
      if [ -n "$__odi_b" ]; then __odi_cur=$__odi_b; else __odi_cur=$(dirname "$__odi_cur"); fi
      if [ "$__odi_rest" = ".." ]; then __odi_rest=; else __odi_rest=${__odi_rest#../}; fi
    done
    [ -n "$__odi_rest" ] && __odi_cur="$__odi_cur/$__odi_rest"
    builtin cd "$__odi_cur"; __odi_s=$?; unset __odi_cur __odi_rest __odi_b; return $__odi_s
  fi
  if [ "$#" -eq 1 ] && [ ! -d "$1" ]; then
    case "$1" in
      *.odi|*.odi/*|*/*:*)
        unset __odi_dir __odi_top __odi_back
        eval "$(command odi open --sh "$1")"
        [ -n "${__odi_dir:-}" ] || return 1
        # (one line per file: the newest for each)
        __ODI_TRAIL=$(printf '%s\n' "${__ODI_TRAIL:-}" | awk -F'|' -v t="$__odi_top" '$1 != "" && $1 != t')
        __ODI_TRAIL="${__ODI_TRAIL:+$__ODI_TRAIL
}${__odi_top}|${__odi_back:-$PWD}"
        builtin cd "$__odi_dir"; __odi_s=$?; unset __odi_dir __odi_top __odi_back; return $__odi_s ;;
    esac
  fi
  builtin cd "$@"
}
# (Tab after cd offers .odi files too, beside folders.)
if [ -n "${ZSH_VERSION:-}" ]; then
  _odi_cd() { local r=1; _cd "$@" && r=0; _files -g '*.odi(-.)' && r=0; return $r; }
  __odi_compdef() { (( $+functions[compdef] )) && compdef _odi_cd cd; add-zsh-hook -d precmd __odi_compdef; }
  autoload -Uz add-zsh-hook && add-zsh-hook precmd __odi_compdef
elif [ -n "${BASH_VERSION:-}" ]; then
  _odi_cd_complete() {
    local IFS=$'\n' cur=${COMP_WORDS[COMP_CWORD]}
    COMPREPLY=($(compgen -d -- "$cur") $(compgen -f -X '!*.odi' -- "$cur"))
  }
  complete -o filenames -o nospace -F _odi_cd_complete cd
fi
"#;

/// The same cd for fish.
const FISH_HOOK: &str = r#"# odi: cd into .odi files (cd app.odi), and hub names with a tag (cd acme/web:1.4)
function __odi_back_of
    for t in $__odi_trail[-1..1]
        set -l p (string split -m1 '|' -- $t)
        test "$p[1]" = "$argv[1]"; and echo $p[2]; and return
    end
end
function cd --wraps cd
    if test (count $argv) -eq 1; and set -q __odi_trail; and string match -qr '^\.\.(/|$)' -- $argv[1]
        set -l inside
        for t in $__odi_trail
            set -l p (string split -m1 '|' -- $t)
            if test "$PWD" = "$p[1]"; or string match -q -- "$p[1]/*" $PWD
                set inside 1
            end
        end
        if test -n "$inside"
            set -l cur $PWD
            set -l rest $argv[1]
            while string match -qr '^\.\.(/|$)' -- $rest
                set -l b (__odi_back_of $cur)
                if test -n "$b"; set cur $b; else; set cur (dirname $cur); end
                set rest (string replace -r '^\.\./?' '' -- $rest)
            end
            test -n "$rest"; and set cur "$cur/$rest"
            builtin cd $cur
            return
        end
    end
    if test (count $argv) -eq 1; and not test -d $argv[1]; and string match -qr '\.odi(/|$)|/.*:' -- $argv[1]
        set -l out (command odi open --lines $argv[1]); or return
        set -l back $PWD
        test -n "$out[3]"; and set back $out[3]
        set -l keep
        for t in $__odi_trail
            string match -q -- "$out[2]|*" $t; or set -a keep $t
        end
        set -g __odi_trail $keep "$out[2]|$back"
        builtin cd $out[1]
        return
    end
    builtin cd $argv
end
complete -c cd -a '(__fish_complete_suffix .odi)'
"#;

/// Whether s is shaped like a hub name, NAMESPACE/NAME:TAG (or @sha256:…):
/// anything else that isn't a file is a mistyped path, and is said so at
/// once, with no network.
fn hub_name(s: &str) -> bool {
    let ok = |w: &str| {
        !w.is_empty()
            && w.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && w.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
    };
    let (repo, rest) = match (s.split_once(':'), s.split_once('@')) {
        (_, Some((r, d))) => (r, d.strip_prefix("sha256:").filter(|d| d.len() == 64).map(|_| "x").unwrap_or("")),
        (Some((r, t)), None) => (r, t),
        (None, None) => return false,
    };
    let tag_ok = !rest.is_empty()
        && rest.len() <= 128
        && rest.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'));
    tag_ok && repo.split_once('/').is_some_and(|(a, b)| ok(a) && ok(b))
}

/// The folder an opened file is at: its name and the start of its key, so
/// a prompt shows ~/.odi/open/shop-3f2a9c1e0b7d, not a hash.
fn folder_for(app: &parts::Assembled) -> PathBuf {
    let name: String = app
        .rd
        .manifest
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    let key = &app.key[..app.key.len().min(12)];
    base().join(if name.is_empty() { key.to_string() } else { format!("{name}-{key}") })
}

/// odi open FILE|NAME[/PATH]: prints a folder that is the file (made if
/// need be): what `cd app.odi` goes to. --sh and --lines (for odi hook's
/// cd) also say the file's top, and where `cd ..` from it goes: the folder
/// the file is in (nothing for a hub name).
pub fn cmd_open(args: &[String]) -> Result<()> {
    let (mode, arg) = match args {
        [m, a] if m == "--sh" || m == "--lines" => (m.as_str(), a),
        [a] => ("", a),
        _ => bail!("usage: odi open [--sh|--lines] FILE|NAME[/PATH]"),
    };
    let (src, sub) = split(arg);
    if !Path::new(&src).exists() && !hub_name(&src) {
        let here = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
        bail!("no file {src} in {here}");
    }
    let (rd, remote) = crate::hub::open_any(&src)?;
    let app = parts::assemble(rd, remote)?;
    let top = open_at(&src, &app, &folder_for(&app))?;
    let at = if sub.is_empty() { top.clone() } else { top.join(&sub) };
    if !at.is_dir() {
        bail!("{sub}: no such folder in {src}");
    }
    let back = if Path::new(&src).is_file() {
        std::fs::canonicalize(&src)?.parent().map(|p| p.display().to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    match mode {
        "--sh" => println!(
            "__odi_dir={}; __odi_top={}; __odi_back={}",
            q(&at.display().to_string()),
            q(&top.display().to_string()),
            q(&back)
        ),
        "--lines" => println!("{}\n{}\n{back}", at.display(), top.display()),
        _ => println!("{}", at.display()),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_at(src: &str, app: &parts::Assembled, dir: &Path) -> Result<PathBuf> {
    if mounted(dir) {
        if std::fs::read_dir(dir).is_ok() {
            return Ok(dir.to_path_buf());
        }
        // (Its odi stopped: "transport endpoint is not connected". Let the
        // dead mount go, and mount it again.)
        unmount_lazy(dir);
    }
    if unpacked(dir) {
        return Ok(dir.to_path_buf());
    }
    if Path::new("/dev/fuse").exists() {
        std::fs::create_dir_all(dir)?;
        match spawn_mount(src, dir) {
            Ok(()) => return Ok(dir.to_path_buf()),
            Err(e) => {
                let _ = std::fs::remove_dir(dir);
                eprintln!("odi: unpacking instead ({e:#})");
            }
        }
    }
    unpack(app, dir)
}

#[cfg(not(target_os = "linux"))]
fn open_at(_src: &str, app: &parts::Assembled, dir: &Path) -> Result<PathBuf> {
    if unpacked(dir) {
        return Ok(dir.to_path_buf());
    }
    unpack(app, dir)
}

fn unpacked(dir: &Path) -> bool {
    dir.join(".odi-open").exists()
}

/// The file unpacked into dir, read-only (anyone changing it would change
/// what every later `cd` there sees).
fn unpack(app: &parts::Assembled, dir: &Path) -> Result<PathBuf> {
    if app.remote.is_some() {
        eprintln!("odi: getting the whole file");
    }
    let tmp = dir.with_extension(format!("tmp-{}", std::process::id()));
    remove_tree(&tmp);
    if let Err(e) = crate::extract(&app.rd, &tmp) {
        remove_tree(&tmp);
        return Err(e);
    }
    crate::run::sync_tree(&tmp);
    std::fs::write(tmp.join(".odi-open"), b"")?;
    set_writable(&tmp, false);
    // (Another terminal may have opened the same file meanwhile, and be in
    // it: a finished copy is never replaced. Only a broken one, from a run
    // that stopped half way, is.)
    if unpacked(dir) {
        remove_tree(&tmp);
        return Ok(dir.to_path_buf());
    }
    if let Err(e) = std::fs::rename(&tmp, dir) {
        if unpacked(dir) {
            remove_tree(&tmp);
            return Ok(dir.to_path_buf());
        }
        remove_tree(dir);
        std::fs::rename(&tmp, dir).map_err(|_| e).with_context(|| dir.display().to_string())?;
    }
    Ok(dir.to_path_buf())
}

/// Every file and folder under dir (not through links) made read-only, or
/// writable by its owner again.
fn set_writable(dir: &Path, on: bool) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        if on {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(meta.permissions().mode() | 0o200));
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                set_writable(&e.path(), on);
            }
        }
    }
    let mode = meta.permissions().mode();
    let mode = if on { mode | 0o200 } else { mode & !0o222 };
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
}

fn remove_tree(dir: &Path) {
    if dir.exists() {
        set_writable(dir, true);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// odi close [FILE|NAME]: unmounts what `cd` opened (or that one), and
/// removes unpacked copies.
pub fn cmd_close(args: &[String]) -> Result<()> {
    let only = match args {
        [] => None,
        [one] => {
            let (src, _) = split(one);
            let (rd, remote) = crate::hub::open_any(&src)?;
            Some(folder_for(&parts::assemble(rd, remote)?))
        }
        _ => bail!("usage: odi close [FILE|NAME]"),
    };
    let Ok(rd) = std::fs::read_dir(base()) else {
        println!("nothing open");
        return Ok(());
    };
    let (mut closed, mut busy) = (0, Vec::new());
    for e in rd.flatten() {
        let dir = e.path();
        if only.as_ref().is_some_and(|o| *o != dir) || dir.extension().is_some_and(|x| x == "log") {
            continue;
        }
        if mounted(&dir) {
            #[cfg(target_os = "linux")]
            if std::fs::read_dir(&dir).is_err() {
                unmount_lazy(&dir);
                closed += 1;
                let _ = std::fs::remove_dir(&dir);
                continue;
            }
            if unmount(&dir) {
                closed += 1;
                let _ = std::fs::remove_dir(&dir);
            } else {
                busy.push(dir);
            }
        } else if dir.is_dir() {
            remove_tree(&dir);
            closed += 1;
        }
    }
    println!("closed {closed}");
    if !busy.is_empty() {
        bail!(
            "in use (a shell or program is in it: cd out, then odi close again): {}",
            busy.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn mounted(dir: &Path) -> bool {
    let want = dir.display().to_string().replace(' ', "\\040");
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|m| m.lines().any(|l| l.split(' ').nth(4) == Some(want.as_str())))
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn mounted(_dir: &Path) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn unmount(dir: &Path) -> bool {
    ["fusermount3", "fusermount"].iter().any(|p| {
        std::process::Command::new(p)
            .arg("-u")
            .arg(dir)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[cfg(not(target_os = "linux"))]
fn unmount(_dir: &Path) -> bool {
    false
}

/// A mount let go even while in use or broken (it goes once nothing uses it).
#[cfg(target_os = "linux")]
fn unmount_lazy(dir: &Path) {
    for p in ["fusermount3", "fusermount"] {
        let st = std::process::Command::new(p).arg("-uz").arg(dir).stderr(std::process::Stdio::null()).status();
        if st.is_ok_and(|s| s.success()) {
            return;
        }
    }
}

/// Starts an odi in the background that keeps src mounted at dir (until
/// odi close), and waits until it is.
#[cfg(target_os = "linux")]
fn spawn_mount(src: &str, dir: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let log = dir.with_extension("log");
    let out = std::fs::File::create(&log)?;
    // (From /, so it never keeps busy the folder it was started in, which
    // may be another opened file; a relative path made whole first.)
    let src = match std::fs::canonicalize(src) {
        Ok(p) if Path::new(src).exists() => p.display().to_string(),
        _ => src.to_string(),
    };
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.current_dir("/").arg("__mount").arg(&src).arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(out);
    // SAFETY: setsid(2) only detaches the child from this terminal's session.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(60) {
        if mounted(dir) {
            return Ok(());
        }
        if let Some(st) = child.try_wait()? {
            let why = std::fs::read_to_string(&log).unwrap_or_default();
            bail!("{}", why.trim().strip_prefix("odi: ").unwrap_or(why.trim()).to_string() + &format!(" ({st})"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    Err(anyhow!("mounting took over a minute"))
}

/// odi __mount SRC DIR: the background odi behind `cd`: mounts, then waits
/// until it's unmounted (odi close) or told to stop.
#[cfg(target_os = "linux")]
pub fn cmd_mount_daemon(args: &[String]) -> Result<()> {
    let [src, dir] = args else { bail!("usage: odi __mount SRC DIR") };
    let dir = PathBuf::from(dir);
    let (rd, remote) = crate::hub::open_any(src)?;
    let app = parts::assemble(rd, remote)?;
    let scratch = tempfile::Builder::new().prefix("odi-open-").tempdir()?;
    let m = crate::mount::mount_at(app.rd.clone(), &scratch.path().join(".writable"), app.remote.clone(), &dir)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).ok();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    if !mounted(&dir) { break; }
                }
                _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending().await } } => break,
            }
        }
    });
    drop(m);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn cmd_mount_daemon(_args: &[String]) -> Result<()> {
    bail!("mounting is on Linux for now")
}

#[cfg(test)]
mod tests {
    use super::{hub_name, split};

    #[test]
    fn splits() {
        let s = |a: &str| split(a);
        assert_eq!(s("app.odi"), ("app.odi".into(), "".into()));
        assert_eq!(s("app.odi/"), ("app.odi".into(), "".into()));
        assert_eq!(s("dir/app.odi/app/lib"), ("dir/app.odi".into(), "app/lib".into()));
        assert_eq!(s("acme/web:1.4"), ("acme/web:1.4".into(), "".into()));
        assert_eq!(s("acme/web:1.4/app"), ("acme/web:1.4".into(), "app".into()));
    }

    #[test]
    fn hub_names() {
        for yes in ["acme/web:1.4", "acme/web:latest", "a/b:1.0.0-rc.1", &format!("acme/web@sha256:{}", "a".repeat(64))]
        {
            assert!(hub_name(yes), "{yes}");
        }
        for no in ["notes:old", "./notes:old", "my notes/x:1", "Acme/web:1", "acme/web", "acme/web:", "a/b/c:1", "C:/x"]
        {
            assert!(!hub_name(no), "{no}");
        }
    }
}
