//! Pontos de falha do export/render (feature `failpoints`; **fora** do build normal):
//! `CAPIA_FAILPOINT=<ponto>` + `CAPIA_FAILPOINT_MODE=abort|park`.

#[cfg(feature = "failpoints")]
pub(crate) fn hit(name: &str) {
    use std::io::Write as _;
    if std::env::var("CAPIA_FAILPOINT").ok().as_deref() != Some(name) {
        return;
    }
    match std::env::var("CAPIA_FAILPOINT_MODE").ok().as_deref() {
        Some("park") => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "FAILPOINT_REACHED {name}");
            let _ = out.flush();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        _ => std::process::abort(),
    }
}

macro_rules! fp {
    ($name:expr) => {
        #[cfg(feature = "failpoints")]
        $crate::failpoints::hit($name);
    };
}
pub(crate) use fp;
