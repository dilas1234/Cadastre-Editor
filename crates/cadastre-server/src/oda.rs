//! DWG → DXF through the ODA File Converter (Open Design Alliance, free), for the Checking and
//! Data Verification unit's CAD import. DWG is a closed format; the converter is the dependable way
//! to read every AutoCAD version without linking a DWG library into the server.
//!
//! The converter is found from `CADASTRE_ODA_CONVERTER` (the program, or a wrapper script), else
//! `ODAFileConverter` on the `PATH`. It is a Qt program that ships only Qt's `xcb` plugin (27.10), so
//! outside Windows it needs an X display even with no window to show: it is run under `xvfb-run` (a
//! virtual display), or on the server's own `DISPLAY` when `xvfb-run` is not installed.
//!
//! Command line: `ODAFileConverter <in dir> <out dir> <version> <type> <recurse> <audit> [filter]`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

/// How long one conversion may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// One conversion at a time: each has its own virtual display.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Why a DWG could not be converted. The text is shown to the officer.
#[derive(Debug)]
pub enum OdaError {
    /// No converter on this server.
    NotInstalled,
    /// The converter is there, but neither a virtual nor a real display is.
    NoDisplay,
    /// The converter ran but produced no DXF (or failed to start, or timed out).
    Failed(String),
}

impl std::fmt::Display for OdaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const UNTIL: &str = "Until then, save the drawing as DXF in AutoCAD and import the DXF.";
        match self {
            OdaError::NotInstalled => write!(f, "DWG files need the ODA File Converter on the server, and it is not installed \
                (see docs/checking-and-data-verification.md). {UNTIL}"),
            OdaError::NoDisplay => write!(f, "DWG files need a virtual display on the server for the ODA File Converter, and it is not installed: \
                ask the ICT Unit to install the xvfb package there. {UNTIL}"),
            OdaError::Failed(why) => write!(f, "the DWG could not be converted: {why}. Save the drawing as DXF in AutoCAD and import the DXF."),
        }
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([PathBuf::from("/usr/bin").join(name), PathBuf::from("/usr/local/bin").join(name)])
        .find(|p| p.is_file())
}

/// The converter to run, if this server has one.
pub fn converter() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CADASTRE_ODA_CONVERTER").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    on_path(if cfg!(windows) { "ODAFileConverter.exe" } else { "ODAFileConverter" })
}

/// Where the converter's window system comes from.
#[derive(Debug, Clone, PartialEq)]
enum Screen {
    /// As the server runs (Windows, or a Linux server with its own `DISPLAY`).
    Own,
    /// Under this `xvfb-run`.
    Virtual(PathBuf),
}

/// A virtual display when there is one: the server's own `DISPLAY` is a desktop session's, which
/// would flash a window there and is gone when that person signs out.
fn choose_screen(windows: bool, xvfb_run: Option<PathBuf>, has_display: bool) -> Result<Screen, OdaError> {
    match xvfb_run {
        _ if windows => Ok(Screen::Own),
        Some(x) => Ok(Screen::Virtual(x)),
        None if has_display => Ok(Screen::Own),
        None => Err(OdaError::NoDisplay),
    }
}

/// Convert one DWG to ASCII DXF (AutoCAD 2018 format) and return the DXF's bytes.
pub async fn dwg_to_dxf(dwg: &[u8]) -> Result<Vec<u8>, OdaError> {
    let program = converter().ok_or(OdaError::NotInstalled)?;
    let has_display = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
    let screen = choose_screen(cfg!(windows), on_path("xvfb-run"), has_display)?;
    let dwg = dwg.to_vec();
    // Its own task: a browser that goes away must not leave the converter or its files behind.
    tokio::spawn(async move {
        let _one = ONE_AT_A_TIME.lock().await;
        let work = std::env::temp_dir().join(format!("cadastre-dwg-{}", uuid::Uuid::new_v4()));
        let result = convert_in(&program, &screen, &work, &dwg, TIMEOUT).await;
        let _ = tokio::fs::remove_dir_all(&work).await;
        result
    })
    .await
    .unwrap_or_else(|e| Err(OdaError::Failed(format!("the conversion stopped: {e}"))))
}

/// The converter's own words on one line, without this server's folders.
fn reason(text: &str, input: &Path, output: &Path, work: &Path) -> String {
    let mut t = text.to_string();
    for dir in [input, output, work] {
        let d = dir.to_string_lossy();
        t = t.replace(&format!("{d}{}", std::path::MAIN_SEPARATOR), "").replace(&*d, "");
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect::<String>().trim_end_matches('.').to_string()
}

/// Stop everything the conversion started: `xvfb-run` is a script, so ending only it would leave
/// the converter and the virtual display running. They share the process group made for them.
#[cfg(unix)]
async fn stop_group(pid: u32) {
    for signal in ["-TERM", "-KILL"] {
        let _ = Command::new("kill").args([signal, "--", &format!("-{pid}")]).stderr(std::process::Stdio::null()).status().await;
        tokio::time::sleep(Duration::from_secs(1)).await; // TERM: let xvfb-run clear up its display
    }
}

async fn convert_in(program: &Path, screen: &Screen, work: &Path, dwg: &[u8], limit: Duration) -> Result<Vec<u8>, OdaError> {
    let (input, output) = (work.join("in"), work.join("out"));
    let io = |e: std::io::Error| OdaError::Failed(format!("temporary files: {e}"));
    tokio::fs::create_dir_all(&input).await.map_err(io)?;
    tokio::fs::create_dir_all(&output).await.map_err(io)?;
    tokio::fs::write(input.join("DRAWING.DWG"), dwg).await.map_err(io)?;
    let mut cmd = match screen {
        Screen::Own => Command::new(program),
        Screen::Virtual(xvfb_run) => {
            let mut c = Command::new(xvfb_run);
            c.arg("-a").arg(program);
            c
        }
    };
    cmd.arg(&input).arg(&output).args(["ACAD2018", "DXF", "0", "1", "*.DWG"])
        // Whatever the server's own session uses (wayland, offscreen), the converter has only xcb.
        .env_remove("QT_QPA_PLATFORM").env_remove("WAYLAND_DISPLAY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn().map_err(|e| {
        tracing::warn!("ODA File Converter {} ({screen:?}): {e}", program.display());
        OdaError::Failed(format!("the converter could not be started ({e})"))
    })?;
    #[cfg(unix)]
    let pid = child.id();
    let waiting = child.wait_with_output();
    tokio::pin!(waiting);
    let run = match tokio::time::timeout(limit, &mut waiting).await {
        Ok(r) => r.map_err(|e| OdaError::Failed(format!("the converter could not be run ({e})")))?,
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = pid { stop_group(pid).await; }
            return Err(OdaError::Failed(format!("the converter took longer than {} s", limit.as_secs())));
        }
    };
    // The output keeps the input's name; its extension's case is the converter's. A drawing it
    // cannot read is reported in a `.err` file beside it, not by the exit code.
    let (mut dxf, mut why) = (Vec::new(), String::new());
    if let Ok(mut dir) = tokio::fs::read_dir(&output).await {
        while let Ok(Some(e)) = dir.next_entry().await {
            let path = e.path();
            match path.extension().map(|x| x.to_ascii_lowercase()) {
                Some(x) if x == "dxf" => dxf = tokio::fs::read(&path).await.unwrap_or_default(),
                Some(x) if x == "err" => why = String::from_utf8_lossy(&tokio::fs::read(&path).await.unwrap_or_default()).into_owned(),
                _ => {}
            }
        }
    }
    if !dxf.is_empty() {
        return Ok(dxf);
    }
    if why.trim().is_empty() {
        // xvfb-run may pass the converter's errors on as output.
        why = format!("{} {}", String::from_utf8_lossy(&run.stderr), String::from_utf8_lossy(&run.stdout));
    }
    let why = reason(&why, &input, &output, work);
    tracing::warn!("ODA File Converter {} ({screen:?}) gave no DXF, {}: {why}", program.display(), run.status);
    Err(OdaError::Failed(if why.is_empty() { format!("no DXF came out ({})", run.status) } else { why }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_converter_gets_a_virtual_display() {
        let x = PathBuf::from("/usr/bin/xvfb-run");
        assert_eq!(choose_screen(false, Some(x.clone()), false).unwrap(), Screen::Virtual(x.clone()));
        assert_eq!(choose_screen(false, Some(x.clone()), true).unwrap(), Screen::Virtual(x), "not somebody's desktop");
        assert_eq!(choose_screen(false, None, true).unwrap(), Screen::Own);
        assert_eq!(choose_screen(true, None, false).unwrap(), Screen::Own);
        let e = choose_screen(false, None, false).unwrap_err().to_string();
        assert!(e.contains("xvfb") && e.contains("DXF"), "{e}");
    }

    /// Stand-ins (shell scripts) prove the command line and the file hand-over; the real
    /// converter's behaviour is `the_real_converter_reads_a_dwg`.
    #[cfg(unix)]
    #[tokio::test]
    async fn runs_the_converter_and_reads_its_dxf() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("oda-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        // As the real one: no offscreen plugin, and no display without xvfb-run.
        let fake = script("ODAFileConverter", "[ -z \"$QT_QPA_PLATFORM\" ] || exit 4\n[ \"$DISPLAY\" = :virtual ] || { echo no display; exit 5; }\n\
            [ \"$3 $4 $5 $6 $7\" = \"ACAD2018 DXF 0 1 *.DWG\" ] || exit 3\ncp \"$1/DRAWING.DWG\" \"$2/DRAWING.dxf\"");
        let xvfb = Screen::Virtual(script("xvfb-run", "[ \"$1\" = -a ] || exit 6\nshift\nDISPLAY=:virtual exec \"$@\""));
        let out = convert_in(&fake, &xvfb, &dir.join("w1"), b"0\nSECTION\n", TIMEOUT).await.unwrap();
        assert_eq!(out, b"0\nSECTION\n");

        // A failure reported on output, as through xvfb-run.
        let dies = script("dies", "echo 'no Qt platform plugin'; exit 134");
        let err = convert_in(&dies, &Screen::Own, &dir.join("w2"), b"x", TIMEOUT).await.unwrap_err().to_string();
        assert!(err.contains("no Qt platform plugin"), "{err}");

        // An .err file gives the reason, without the server's folders.
        let bad = script("bad", "echo \"Error in drawing \\\"$1/DRAWING.DWG\\\" : Unexpected end of file\" > \"$2/DRAWING.dxf.err\"");
        let err = convert_in(&bad, &Screen::Own, &dir.join("w3"), b"x", TIMEOUT).await.unwrap_err().to_string();
        assert!(err.contains("\"DRAWING.DWG\" : Unexpected end of file") && !err.contains(&*dir.to_string_lossy()), "{err}");

        // Too slow: what it started is stopped too, not only the script.
        let slow = script("slow", &format!("sleep 600 & echo $! > '{}'\nwait", dir.join("pid").display()));
        let err = convert_in(&slow, &Screen::Own, &dir.join("w4"), b"x", Duration::from_secs(1)).await.unwrap_err().to_string();
        assert!(err.contains("longer than 1 s"), "{err}");
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        // Ended, or a zombie waiting for its new parent.
        let state = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())).unwrap_or_default();
        assert!(state.is_empty() || state.contains(") Z "), "still running: {state}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// With the real converter (skipped where it, or a display for it, is missing): a DXF is made
    /// into a DWG by the converter and read back through the server's own path.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_real_converter_reads_a_dwg() {
        let Some(program) = on_path("ODAFileConverter") else {
            eprintln!("skipping: ODAFileConverter not installed");
            return;
        };
        let display = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
        if choose_screen(false, on_path("xvfb-run"), display).is_err() {
            eprintln!("skipping: no xvfb-run and no DISPLAY for ODAFileConverter");
            return;
        }
        let screen = choose_screen(false, on_path("xvfb-run"), display).unwrap();
        let dir = std::env::temp_dir().join(format!("oda-real-{}", uuid::Uuid::new_v4()));
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("plan.dxf"), "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nBOUNDARY\n10\n493000.125\n20\n1083100.5\n30\n0.0\n11\n493030.0\n21\n1083100.5\n31\n0.0\n0\nENDSEC\n0\nEOF\n").unwrap();
        let mut cmd = match &screen {
            Screen::Own => Command::new(&program),
            Screen::Virtual(x) => { let mut c = Command::new(x); c.arg("-a").arg(&program); c }
        };
        cmd.arg(&src).arg(&dst).args(["ACAD2018", "DWG", "0", "1", "*.DXF"]).env_remove("QT_QPA_PLATFORM").env_remove("WAYLAND_DISPLAY");
        cmd.output().await.unwrap();
        let dwg = std::fs::read(dst.join("plan.dwg")).expect("the converter made a DWG");
        assert!(dwg.starts_with(b"AC1032"));
        let dxf = convert_in(&program, &screen, &dir.join("work"), &dwg, TIMEOUT).await.unwrap();
        let d = cadastre_import::dxf::parse_bytes(&dxf).unwrap();
        assert_eq!((d.lines.len(), d.lines[0].layer.as_str()), (1, "BOUNDARY"));
        assert!((d.lines[0].from.x - 493000.125).abs() < 1e-6 && (d.lines[0].from.y - 1083100.5).abs() < 1e-6);
        let err = convert_in(&program, &screen, &dir.join("bad"), b"AC1032 not a real drawing", TIMEOUT).await.unwrap_err().to_string();
        assert!(err.contains("Unexpected end of file") && !err.contains(&*dir.to_string_lossy()), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
