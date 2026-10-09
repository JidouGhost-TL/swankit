//! Installed-wheel parity through Formatkit's serialized frozen-spec harness.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{run_dir, Args, Digest};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Worker {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // This worker was started by this test; no unrelated processes are touched.
        let _ = writeln!(self.input, "null");
        let _ = self.input.flush();
        let _ = self.child.wait();
    }
}
struct PythonDigest(RefCell<Worker>);
impl Digest for PythonDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let io_error = |e: std::io::Error| Error::Malformed(format!("Python worker: {e}"));
        let mut worker = self.0.borrow_mut();
        // Transport only: the Python extension still performs every format read.
        let request = json!({"data": bytes, "args": args.0});
        serde_json::to_writer(&mut worker.input, &request)
            .map_err(|e| Error::Malformed(e.to_string()))?;
        writeln!(worker.input).map_err(io_error)?;
        worker.input.flush().map_err(io_error)?;
        let mut line = String::new();
        worker.output.read_line(&mut line).map_err(io_error)?;
        let response: Value = serde_json::from_str(&line)
            .map_err(|e| Error::Malformed(format!("Python worker response: {e}")))?;
        if let Some(error) = response.get("error") {
            return Err(Error::Malformed(format!("Python binding: {error}")));
        }
        Ok(response)
    }
}
#[test]
fn corpus_python_bindings() {
    if std::env::var_os("FORMATKIT_CORPUS_DIR").is_none() {
        assert_ne!(
            std::env::var("FORMATKIT_CORPUS_REQUIRED").as_deref(),
            Ok("1")
        );
        return;
    }
    assert_eq!(
        std::env::var("FORMATKIT_CORPUS_RUNNER_LOCKED").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("FORMATKIT_CORPUS_REQUIRED").as_deref(),
        Ok("1")
    );
    assert_ne!(std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(), Ok("1"));
    let mut child = Command::new("python")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/corpus_worker.py"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("container Python with installed swankit wheel");
    let input = BufWriter::new(child.stdin.take().expect("worker stdin"));
    let output = BufReader::new(child.stdout.take().expect("worker stdout"));
    let handler = PythonDigest(RefCell::new(Worker {
        child,
        input,
        output,
    }));
    let count = run_dir("swankit-python", &handler);
    eprintln!("Python installed wheel: {count} frozen owner samples passed");
    drop(handler);
    if let Some(script) = std::env::var_os("SWANKIT_PYTHON_DEMO") {
        assert!(Command::new("python")
            .arg(script)
            .status()
            .expect("external parity demo")
            .success());
    }
}
