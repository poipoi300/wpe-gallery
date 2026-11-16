//! Make-before-break renderer supervision.
//!
//! The launcher stays the unit's main process and owns one renderer at a time.
//! SIGHUP (`systemctl reload`) re-verifies the assignments and starts a second
//! renderer; the old one is killed only once the new one has presented frames,
//! so the desktop never falls back to the bare background between wallpapers.
//! Wayland compositors (niri, wlroots) stack a newer layer surface above an
//! older one in the same layer, so the new wallpaper covers the old as soon as
//! it maps and killing the old one is invisible.
//!
//! Readiness comes from the renderer's own `--screenshot`: it captures after
//! `--screenshot-delay` (at most 5) Wayland dispatch rounds, which only happen
//! once the compositor answers the first frames. Measured under headless sway
//! and nested niri, the file appears 150-180 ms after the first visible frame.
use anyhow::{Context, Result, bail};
use std::{
    fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

/// How often a starting renderer's readiness file is checked.
const POLL: Duration = Duration::from_millis(25);
/// Longest wait before the old renderer is replaced regardless. A renderer
/// paused behind a fullscreen window never draws, so never becomes ready.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(PartialEq)]
enum Event {
    Reload,
    Stop,
}

/// A running renderer. Dropping it kills and reaps the process, so no exit
/// path of the supervisor can leave an orphaned renderer holding GPU memory.
struct Instance {
    child: Child,
    ready: PathBuf,
    started: Instant,
}

impl Instance {
    /// Spawns `command`, which signals readiness by creating `ready`.
    fn spawn(mut command: Command, ready: PathBuf) -> Result<Self> {
        let parent = std::process::id();
        // SAFETY: the hook only calls async-signal-safe libc functions.
        unsafe {
            command.pre_exec(move || {
                // The renderer dies with the supervisor even outside systemd.
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent {
                    libc::_exit(1);
                }
                // The supervisor blocks the signals it waits for, and children
                // inherit the mask: the renderer must start with none blocked.
                let mut set = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                match libc::pthread_sigmask(libc::SIG_SETMASK, &set, std::ptr::null_mut()) {
                    0 => Ok(()),
                    error => Err(io::Error::from_raw_os_error(error)),
                }
            });
        }
        let child = command.spawn().context("start renderer")?;
        Ok(Self {
            child,
            ready,
            started: Instant::now(),
        })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        _ = self.child.kill();
        _ = self.child.wait();
        _ = fs::remove_file(&self.ready);
    }
}

/// Supervises renderers until a stop request or an unreplaced renderer exits.
///
/// `spawn` starts a verified renderer; `wait` blocks for events, for at most
/// the given timeout, and also returns early when a child exits.
fn run<S, W>(mut spawn: S, mut wait: W) -> Result<()>
where
    S: FnMut() -> Result<Instance>,
    W: FnMut(Option<Duration>) -> Result<Vec<Event>>,
{
    let mut current: Option<Instance> = None;
    let mut pending = Some(spawn()?);
    loop {
        let events = wait(pending.as_ref().map(|_| POLL))?;
        if events.contains(&Event::Stop) {
            return Ok(());
        }
        // Settle first, so a replacement that is already presenting is kept
        // rather than discarded by a newer request.
        let reload = events.contains(&Event::Reload);
        if let Some(status) = settle(&mut current, &mut pending)? {
            if !reload {
                return finished(status);
            }
            eprintln!(
                "wallpaperengine-start: renderer exited ({status}); starting the requested one"
            );
        }
        if reload {
            // A newer request supersedes a renderer still starting.
            drop(pending.take());
            // Like a restart, a failed verification stops everything.
            pending = Some(spawn()?);
        }
    }
}

/// Reaps exited renderers and promotes a ready replacement, killing the one
/// it replaces. Returns the exit status once no renderer is left.
fn settle(
    current: &mut Option<Instance>,
    pending: &mut Option<Instance>,
) -> Result<Option<ExitStatus>> {
    if let Some(shown) = current.as_mut()
        && let Some(status) = shown.child.try_wait()?
    {
        *current = None;
        if pending.is_none() {
            return Ok(Some(status));
        }
        eprintln!("wallpaperengine-start: renderer exited ({status}) while its replacement starts");
    }
    let Some(next) = pending.as_mut() else {
        return Ok(None);
    };
    if let Some(status) = next.child.try_wait()? {
        *pending = None;
        if current.is_none() {
            return Ok(Some(status));
        }
        eprintln!(
            "wallpaperengine-start: new renderer exited before presenting ({status}); keeping the current one"
        );
        return Ok(None);
    }
    let ready = next.ready.exists();
    if !ready && next.started.elapsed() < READY_TIMEOUT {
        return Ok(None);
    }
    if !ready {
        eprintln!(
            "wallpaperengine-start: new renderer not ready after {READY_TIMEOUT:?}; replacing anyway"
        );
    }
    // Dropping the previous renderer kills it.
    *current = pending.take();
    if let Some(shown) = current {
        _ = fs::remove_file(&shown.ready);
    }
    Ok(None)
}

fn finished(status: ExitStatus) -> Result<()> {
    if !status.success() {
        bail!("renderer exited: {status}");
    }
    Ok(())
}

/// Blocked signals consumed synchronously, so none can arrive mid-update.
struct Signals(libc::sigset_t);

impl Signals {
    fn block() -> io::Result<Self> {
        // SAFETY: plain sigset manipulation on a local value.
        unsafe {
            let mut set = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for signal in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT, libc::SIGCHLD] {
                libc::sigaddset(&mut set, signal);
            }
            match libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) {
                0 => Ok(Self(set)),
                error => Err(io::Error::from_raw_os_error(error)),
            }
        }
    }
    fn wait(&self, timeout: Option<Duration>) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut timeout = timeout.map(timespec);
        loop {
            // SAFETY: the set is initialized and the info pointer may be null.
            let signal = unsafe {
                match &timeout {
                    Some(t) => libc::sigtimedwait(&self.0, std::ptr::null_mut(), t),
                    None => libc::sigwaitinfo(&self.0, std::ptr::null_mut()),
                }
            };
            if signal < 0 {
                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EAGAIN) => return Ok(events),
                    Some(libc::EINTR) => continue,
                    _ => return Err(error).context("wait for signals"),
                }
            }
            match signal {
                libc::SIGHUP => events.push(Event::Reload),
                libc::SIGTERM | libc::SIGINT => events.push(Event::Stop),
                _ => {}
            }
            // Drain whatever else is already pending, then return.
            timeout = Some(timespec(Duration::ZERO));
        }
    }
}

fn timespec(d: Duration) -> libc::timespec {
    libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: d.subsec_nanos() as libc::c_long,
    }
}

/// Verifies the assignments and starts a renderer that reports readiness.
fn spawn_renderer(dir: &Path, generation: u64) -> Result<Instance> {
    let args = super::arguments(true)?;
    // BMP: the cheapest format to write for a one-off full-output capture.
    let ready = dir.join(format!("ready-{generation}.bmp"));
    let mut command = Command::new(&args[0]);
    command
        .args(&args[1..])
        .arg("--screenshot")
        .arg(&ready)
        .args(["--screenshot-delay", "5"]);
    Instance::spawn(command, ready)
}

pub fn launch() -> Result<()> {
    let signals = Signals::block().context("block signals")?;
    // A private directory: the renderer writes whatever path it is given.
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let dir = tempfile::Builder::new()
        .prefix("wpe-gallery-")
        .tempdir_in(base)
        .context("create readiness directory")?;
    let mut generation = 0;
    run(
        || {
            generation += 1;
            spawn_renderer(dir.path(), generation)
        },
        |timeout| signals.wait(timeout),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A fake renderer: records its pid, becomes ready after `delay` seconds
    /// (or exits with `code` if given), then idles. `drawn-N` precedes the
    /// readiness file and `shown-N` follows it, so `shown-N` without `ready-N`
    /// means promoted.
    fn fake(dir: &Path, generation: usize, delay: &str, code: Option<i32>) -> Result<Instance> {
        let ready = dir.join(format!("ready-{generation}"));
        let script = match code {
            Some(code) => format!("echo $$ > pid-{generation}; sleep {delay}; exit {code}"),
            None => format!(
                "echo $$ > pid-{generation}; sleep {delay}; : > drawn-{generation}; : > \"$1\"; : > shown-{generation}; exec sleep 60"
            ),
        };
        let mut command = Command::new("sh");
        command
            .current_dir(dir)
            .args(["-c", &script, "sh"])
            .arg(&ready);
        Instance::spawn(command, ready)
    }
    fn pid(dir: &Path, generation: usize) -> Option<i32> {
        fs::read_to_string(dir.join(format!("pid-{generation}")))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks for existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }
    fn promoted(dir: &Path, generation: usize) -> bool {
        dir.join(format!("shown-{generation}")).exists()
            && !dir.join(format!("ready-{generation}")).exists()
    }
    fn wait_for(cond: impl Fn() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    /// A `wait` that polls instead of blocking and gives up after 10 s.
    fn poll(
        mut decide: impl FnMut() -> Vec<Event>,
    ) -> impl FnMut(Option<Duration>) -> Result<Vec<Event>> {
        let start = Instant::now();
        move |timeout| {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(timeout.unwrap_or(POLL));
            Ok(decide())
        }
    }

    #[test]
    fn old_renderer_survives_until_new_one_presents() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let spawned = Cell::new(0);
        let mut reloaded = false;
        run(
            || {
                spawned.set(spawned.get() + 1);
                fake(d, spawned.get(), "0.3", None)
            },
            poll(|| {
                if !reloaded {
                    reloaded = promoted(d, 1);
                    return if reloaded {
                        vec![Event::Reload]
                    } else {
                        vec![]
                    };
                }
                if !d.join("drawn-2").exists() {
                    // Until the new renderer has drawn, the old one must live.
                    assert!(pid(d, 1).is_some_and(alive), "old renderer killed early");
                    return vec![];
                }
                match pid(d, 1).is_some_and(alive) {
                    true => vec![],
                    false => vec![Event::Stop],
                }
            }),
        )
        .unwrap();
        assert_eq!(spawned.get(), 2);
        let p = pid(d, 2).unwrap();
        wait_for(|| !alive(p));
    }

    #[test]
    fn keeps_current_renderer_when_replacement_fails() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let spawned = Cell::new(0);
        let mut reloaded = false;
        run(
            || {
                spawned.set(spawned.get() + 1);
                match spawned.get() {
                    1 => fake(d, 1, "0", None),
                    _ => fake(d, 2, "0", Some(3)),
                }
            },
            poll(|| {
                if !reloaded {
                    reloaded = promoted(d, 1);
                    return if reloaded {
                        vec![Event::Reload]
                    } else {
                        vec![]
                    };
                }
                // Stop once the failed replacement has been reaped.
                match pid(d, 2) {
                    Some(failed) if !alive(failed) => {
                        assert!(pid(d, 1).is_some_and(alive), "current renderer was killed");
                        vec![Event::Stop]
                    }
                    _ => vec![],
                }
            }),
        )
        .unwrap();
        let p = pid(d, 1).unwrap();
        wait_for(|| !alive(p));
    }

    #[test]
    fn reports_renderer_failure() {
        let dir = tempfile::tempdir().unwrap();
        let error = run(|| fake(dir.path(), 1, "0", Some(7)), poll(Vec::new)).unwrap_err();
        assert!(format!("{error}").contains("exit status: 7"), "{error}");
    }

    #[test]
    fn failed_verification_on_reload_stops_current_renderer() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let spawned = Cell::new(0);
        let error = run(
            || {
                spawned.set(spawned.get() + 1);
                match spawned.get() {
                    1 => fake(d, 1, "0", None),
                    _ => bail!("no reviewed-safe assignment"),
                }
            },
            poll(|| match promoted(d, 1) {
                true => vec![Event::Reload],
                false => vec![],
            }),
        )
        .unwrap_err();
        assert!(format!("{error}").contains("reviewed-safe"));
        let p = pid(d, 1).unwrap();
        wait_for(|| !alive(p));
    }
}
