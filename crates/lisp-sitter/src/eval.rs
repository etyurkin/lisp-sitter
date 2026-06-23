use std::process::Command;

use lisp_sitter_core::Error;

/// Run an evaluator for the given Lisp file and return its output.
///
/// Language is inferred from the file extension:
///   - `.el`     → `emacs --batch -f batch-byte-compile`
///   - `.lisp`/`.cl`  → `sbcl --script`
///   - `.scm`/`.ss`/`.sld` → `guile -s`
///
/// Returns `(stdout, stderr, exit_ok)`.
pub fn eval_file(path: &str) -> Result<(String, String, bool), Error> {
    let mut cmd = find_evaluator(path)?;
    let output = cmd.output().map_err(|e| {
        Error::Message(format!("failed to run evaluator for {path}: {e}"))
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = output.status.success();

    Ok((stdout, stderr, ok))
}

fn find_evaluator(path: &str) -> Result<Command, Error> {
    if path.ends_with(".el") {
        // Emacs Lisp: byte-compile
        let abs = std::fs::canonicalize(path)
            .map_err(|e| Error::Message(format!("cannot resolve {path}: {e}")))?;
        let abs_str = abs.to_string_lossy().to_string();
        let mut cmd = Command::new("emacs");
        cmd.args([
            "--batch",
            "--eval",
            &format!("(byte-compile-file \"{abs_str}\")"),
        ]);
        Ok(cmd)
    } else if path.ends_with(".lisp") || path.ends_with(".cl") {
        let bin = which("sbcl")
            .or_else(|_| which("ccl"))
            .map_err(|_| Error::Message(
                "no Common Lisp evaluator found (tried sbcl, ccl). Install one and try again.".into(),
            ))?;
        let mut cmd = Command::new(bin);
        cmd.arg("--script").arg(path);
        Ok(cmd)
    } else if path.ends_with(".scm") || path.ends_with(".ss") || path.ends_with(".sld") {
        let bin = which("guile")
            .or_else(|_| which("chez"))
            .or_else(|_| which("chicken"))
            .map_err(|_| Error::Message(
                "no Scheme evaluator found (tried guile, chez, chicken). Install one and try again.".into(),
            ))?;
        let mut cmd = Command::new(bin);
        cmd.arg("-s").arg(path);
        Ok(cmd)
    } else {
        Err(Error::NoPlugin(path.to_string()))
    }
}

/// Check if a binary exists on `$PATH`.
fn which(name: &str) -> Result<String, ()> {
    let name_variants = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };

    for var in &name_variants {
        if std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .find(|dir| dir.join(var).exists())
            })
            .is_some()
        {
            return Ok(var.clone());
        }
    }
    Err(())
}
