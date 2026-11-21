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

pub fn cmd_hook(args: &[String]) -> Result<()> {
    let shell = match args {
        [s] => s.as_str(),
        [] => bail!("usage: odi hook zsh|bash|fish   (then: eval \"$(odi hook zsh)\" in ~/.zshrc)"),
        _ => bail!("usage: odi hook zsh|bash|fish"),
    };
    let posix = r#"# odi: cd into .odi files, and hub names with a tag (cd acme/web:1.4)
cd() {
  if [ "$#" -eq 1 ] && [ ! -d "$1" ]; then
    case "$1" in
      *.odi|*.odi/*|*:*)
        __odi_dir="$(command odi open "$1")" || return
        builtin cd "$__odi_dir" || return
        unset __odi_dir
        return ;;
    esac
  fi
  builtin cd "$@"
}
"#;
    let fish = r#"# odi: cd into .odi files, and hub names with a tag (cd acme/web:1.4)
function cd --wraps cd
    if test (count $argv) -eq 1; and not test -d $argv[1]; and string match -qr '\.odi(/|$)|:' -- $argv[1]
        set -l dir (command odi open $argv[1]); or return
        builtin cd $dir
        return
    end
    builtin cd $argv
end
"#;
    match shell {
        "zsh" | "bash" | "sh" => print!("{posix}"),
        "fish" => print!("{fish}"),
        other => bail!("no cd for {other:?}: zsh, bash or fish"),
    }
    Ok(())
}

/// odi open FILE|NAME[/PATH]: prints a folder that is the file (made if
/// need be): what `cd app.odi` goes to.
pub fn cmd_open(args: &[String]) -> Result<()> {
    let [arg] = args else { bail!("usage: odi open FILE|NAME[/PATH]") };
    let (src, sub) = split(arg);
    let (rd, remote) = crate::hub::open_any(&src)?;
    let app = parts::assemble(rd, remote)?;
    let dir = base().join(&app.key);
    let top = open_at(&src, &app, &dir)?;
    let at = if sub.is_empty() { top } else { top.join(&sub) };
    if !at.is_dir() {
        bail!("{sub}: no such folder in {src}");
    }
    println!("{}", at.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_at(src: &str, app: &parts::Assembled, dir: &Path) -> Result<PathBuf> {
    if mounted(dir) {
        return Ok(dir.to_path_buf());
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
    remove_tree(dir);
    std::fs::rename(&tmp, dir).with_context(|| dir.display().to_string())?;
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
            Some(base().join(parts::assemble(rd, remote)?.key))
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
    use super::split;

    #[test]
    fn splits() {
        let s = |a: &str| split(a);
        assert_eq!(s("app.odi"), ("app.odi".into(), "".into()));
        assert_eq!(s("app.odi/"), ("app.odi".into(), "".into()));
        assert_eq!(s("dir/app.odi/app/lib"), ("dir/app.odi".into(), "app/lib".into()));
        assert_eq!(s("acme/web:1.4"), ("acme/web:1.4".into(), "".into()));
        assert_eq!(s("acme/web:1.4/app"), ("acme/web:1.4".into(), "app".into()));
    }
}
