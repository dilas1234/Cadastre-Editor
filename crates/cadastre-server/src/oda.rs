//! DWG → DXF through the ODA File Converter (Open Design Alliance, free), for the Checking and
//! Data Verification unit's CAD import. DWG is a closed format; the converter is the dependable way
//! to read every AutoCAD version without linking a DWG library into the server.
//!
//! The converter is found from `CADASTRE_ODA_CONVERTER` (the program, or a wrapper script such as
//! one that runs it under `xvfb-run`), else `ODAFileConverter` on the `PATH`. On Linux it is a Qt
//! program and is run with `QT_QPA_PLATFORM=offscreen`, so no screen is needed.
//!
//! Command line: `ODAFileConverter <in dir> <out dir> <version> <type> <recurse> <audit> [filter]`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long one conversion may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Why a DWG could not be converted. The text is shown to the officer.
#[derive(Debug)]
pub enum OdaError {
    /// No converter on this server.
    NotInstalled,
    /// The converter ran but produced no DXF (or failed to start, or timed out).
    Failed(String),
}

impl std::fmt::Display for OdaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OdaError::NotInstalled => write!(f, "DWG files need the ODA File Converter on the server, and it is not installed \
                (see docs/checking-and-data-verification.md). Until then, save the drawing as DXF in AutoCAD and import the DXF."),
            OdaError::Failed(why) => write!(f, "the DWG could not be converted: {why}. Save the drawing as DXF in AutoCAD and import the DXF."),
        }
    }
}

/// The converter to run, if this server has one.
pub fn converter() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CADASTRE_ODA_CONVERTER").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let name = if cfg!(windows) { "ODAFileConverter.exe" } else { "ODAFileConverter" };
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([PathBuf::from("/usr/bin").join(name), PathBuf::from("/usr/local/bin").join(name)])
        .find(|p| p.is_file())
}

/// Convert one DWG to ASCII DXF (AutoCAD 2018 format) and return the DXF's bytes.
pub async fn dwg_to_dxf(dwg: &[u8]) -> Result<Vec<u8>, OdaError> {
    let program = converter().ok_or(OdaError::NotInstalled)?;
    let work = std::env::temp_dir().join(format!("cadastre-dwg-{}", uuid::Uuid::new_v4()));
    let result = convert_in(&program, &work, dwg).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    result
}

async fn convert_in(program: &Path, work: &Path, dwg: &[u8]) -> Result<Vec<u8>, OdaError> {
    let (input, output) = (work.join("in"), work.join("out"));
    let io = |e: std::io::Error| OdaError::Failed(format!("temporary files: {e}"));
    tokio::fs::create_dir_all(&input).await.map_err(io)?;
    tokio::fs::create_dir_all(&output).await.map_err(io)?;
    tokio::fs::write(input.join("DRAWING.DWG"), dwg).await.map_err(io)?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg(&input).arg(&output).args(["ACAD2018", "DXF", "0", "1", "*.DWG"])
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let run = tokio::time::timeout(TIMEOUT, cmd.output()).await
        .map_err(|_| OdaError::Failed(format!("the converter took longer than {} s", TIMEOUT.as_secs())))?
        .map_err(|e| OdaError::Failed(format!("the converter could not be started ({}): {e}", program.display())))?;
    // The output keeps the input's name; its extension's case is the converter's.
    let mut dxf_path = None;
    if let Ok(mut dir) = tokio::fs::read_dir(&output).await {
        while let Ok(Some(e)) = dir.next_entry().await {
            if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("dxf")) { dxf_path = Some(e.path()); }
        }
    }
    let dxf = match dxf_path { Some(p) => tokio::fs::read(p).await.ok(), None => None };
    match dxf {
        Some(dxf) if !dxf.is_empty() => Ok(dxf),
        _ => {
            // The converter reports a bad drawing in a `.err` file next to the output, not by its exit code.
            let mut why = String::new();
            if let Ok(mut dir) = tokio::fs::read_dir(&output).await {
                while let Ok(Some(e)) = dir.next_entry().await {
                    if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("err")) {
                        why = tokio::fs::read_to_string(e.path()).await.unwrap_or_default();
                    }
                }
            }
            if why.trim().is_empty() {
                why = String::from_utf8_lossy(&run.stderr).into_owned();
            }
            let why = why.split_whitespace().collect::<Vec<_>>().join(" ");
            Err(OdaError::Failed(if why.is_empty() { format!("no DXF came out (exit status {})", run.status) } else { why.chars().take(300).collect() }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in converter (a shell script) proves the command line and the file hand-over.
    #[cfg(unix)]
    #[tokio::test]
    async fn runs_the_converter_and_reads_its_dxf() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("oda-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("ODAFileConverter");
        // in out ACAD2018 DXF 0 1 *.DWG: "convert" by copying, as the real one names its output.
        std::fs::write(&fake, "#!/bin/sh\n[ \"$3 $4 $5 $6\" = \"ACAD2018 DXF 0 1\" ] || exit 3\n\
            [ \"$QT_QPA_PLATFORM\" = offscreen ] || exit 4\ncp \"$1/DRAWING.DWG\" \"$2/DRAWING.dxf\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let work = dir.join("work");
        let out = convert_in(&fake, &work, b"0\nSECTION\n").await.unwrap();
        assert_eq!(out, b"0\nSECTION\n");

        // A converter that writes only an .err file: its text is the reason given.
        std::fs::write(&fake, "#!/bin/sh\necho 'Error: drawing is damaged' > \"$2/DRAWING.DWG.err\"\n").unwrap();
        let err = convert_in(&fake, &dir.join("work2"), b"x").await.unwrap_err().to_string();
        assert!(err.contains("drawing is damaged"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
