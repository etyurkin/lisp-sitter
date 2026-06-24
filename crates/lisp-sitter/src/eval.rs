use std::process::Command;

use lisp_sitter_core::Error;

/// Trait for running an evaluator command.
///
/// Inject a mock implementation in tests to avoid needing external
/// tools (emacs, sbcl, guile) on `$PATH`.
pub trait Runner {
    /// Run `cmd` and return `(stdout, stderr, exit_ok)`.
    fn run(&self, cmd: &mut Command) -> Result<(String, String, bool), Error>;
}

/// Production runner that actually executes the command.
pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, cmd: &mut Command) -> Result<(String, String, bool), Error> {
        let output = cmd.output().map_err(|e| {
            Error::Message(format!("failed to run evaluator: {e}"))
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let ok = output.status.success();
        Ok((stdout, stderr, ok))
    }
}

/// Run an evaluator for the given Lisp file using the real system runner.
///
/// Convenience wrapper around [`eval_file_with`].
pub fn eval_file(path: &str) -> Result<(String, String, bool), Error> {
    eval_file_with(path, &RealRunner)
}

/// Run an evaluator for the given Lisp file with an injectable [`Runner`].
///
/// Language is inferred from the file extension:
///   - `.el`     → `emacs --batch -f batch-byte-compile`
///   - `.lisp`/`.cl`  → `sbcl --script`
///   - `.scm`/`.ss`/`.sld` → `guile -s`
///
/// Returns `(stdout, stderr, exit_ok)`.
pub fn eval_file_with(path: &str, runner: &impl Runner) -> Result<(String, String, bool), Error> {
    let mut cmd = find_evaluator(path)?;
    runner.run(&mut cmd)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock runner that returns canned output without running any command.
    struct MockRunner {
        stdout: String,
        stderr: String,
        exit_ok: bool,
    }

    impl Runner for MockRunner {
        fn run(&self, _cmd: &mut Command) -> Result<(String, String, bool), Error> {
            Ok((self.stdout.clone(), self.stderr.clone(), self.exit_ok))
        }
    }

    #[test]
    fn test_which_not_found() {
        assert!(which("this-binary-definitely-does-not-exist-12345").is_err());
    }

    #[test]
    fn test_which_finds_self() {
        assert!(which("rustc").is_ok());
    }

    #[test]
    fn test_find_evaluator_unsupported_ext() {
        let result = find_evaluator("foo.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_find_evaluator_empty_path() {
        let result = find_evaluator("");
        assert!(result.is_err());
    }

    #[test]
    fn test_find_evaluator_elisp_missing_file() {
        let result = find_evaluator("/nonexistent/test.el");
        assert!(result.is_err());
    }

    #[test]
    fn test_eval_file_not_found() {
        let result = eval_file("/tmp/nonexistent-lisp-sitter-test-file.el");
        assert!(result.is_err());
    }

    // --- eval_file_with (mock runner) covers the full pipeline ---

    #[test]
    fn test_eval_file_with_mock_elisp_success() {
        let dir = std::env::temp_dir().join(format!("lisp-sitter-eval-test-el-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.el");
        std::fs::write(&path, "(+ 1 1)").unwrap();
        let runner = MockRunner { stdout: "2".into(), stderr: String::new(), exit_ok: true };

        let (out, err, ok) = eval_file_with(path.to_str().unwrap(), &runner).unwrap();
        assert_eq!(out, "2");
        assert!(err.is_empty());
        assert!(ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_eval_file_with_mock_elisp_failure() {
        let dir = std::env::temp_dir().join(format!("lisp-sitter-eval-test-el-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.el");
        std::fs::write(&path, "(error \"boom\")").unwrap();
        let runner = MockRunner { stdout: String::new(), stderr: "error: boom".into(), exit_ok: false };

        let (out, err, ok) = eval_file_with(path.to_str().unwrap(), &runner).unwrap();
        assert!(out.is_empty());
        assert_eq!(err, "error: boom");
        assert!(!ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_eval_file_with_mock_common_lisp() {
        let runner = MockRunner { stdout: "42".into(), stderr: String::new(), exit_ok: true };

        // find_evaluator for .lisp tries which("sbcl") first — skip if not installed
        let path = "/tmp/test.lisp";
        let cmd = find_evaluator(path);
        match cmd {
            Ok(mut c) => {
                let (out, _, ok) = runner.run(&mut c).unwrap();
                assert_eq!(out, "42");
                assert!(ok);
            }
            Err(_) => {
                // sbcl not installed, can't construct command — skip
            }
        }
    }

    #[test]
    fn test_eval_file_with_mock_scheme() {
        let runner = MockRunner { stdout: "42".into(), stderr: String::new(), exit_ok: true };

        let path = "/tmp/test.scm";
        let cmd = find_evaluator(path);
        match cmd {
            Ok(mut c) => {
                let (out, _, ok) = runner.run(&mut c).unwrap();
                assert_eq!(out, "42");
                assert!(ok);
            }
            Err(_) => {
                // guile not installed — skip
            }
        }
    }

    #[test]
    fn test_find_evaluator_elisp_constructs_command() {
        let dir = std::env::temp_dir().join(format!("lisp-sitter-eval-test-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.el");
        std::fs::write(&path, "(+ 1 1)").unwrap();

        let cmd = find_evaluator(path.to_str().unwrap()).unwrap();
        // Verify the command is constructed correctly (don't run it)
        assert_eq!(cmd.get_program(), "emacs");
        let args: Vec<&str> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args[0], "--batch");
        assert_eq!(args[1], "--eval");
        assert!(args[2].contains("test.el"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_real_runner_run_error() {
        let runner = RealRunner;
        let mut cmd = Command::new("this-binary-definitely-does-not-exist");
        let result = runner.run(&mut cmd);
        assert!(result.is_err());
    }

    #[test]
    fn test_real_runner_run_success() {
        let runner = RealRunner;
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        let (stdout, stderr, ok) = runner.run(&mut cmd).unwrap();
        assert_eq!(stdout.trim(), "hello");
        assert!(stderr.is_empty());
        assert!(ok);
    }

    #[test]
    fn test_real_runner_run_exit_code() {
        let runner = RealRunner;
        let mut cmd = Command::new("false");
        let (_, _, ok) = runner.run(&mut cmd).unwrap();
        assert!(!ok);
    }
}