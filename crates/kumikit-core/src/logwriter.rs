//! The core's log writer: every formatted line passes through the kernel's secret
//! redaction (`docs/spec/03-kernel.md#secrets`) before it reaches stderr.

use std::io::Write;

/// Makes one [`RedactingLine`] per log event.
#[derive(Debug, Clone, Copy, Default)]
pub struct RedactingStderr;

/// Collects one formatted event and writes it, redacted, when dropped.
#[derive(Debug, Default)]
pub struct RedactingLine(Vec<u8>);

impl Write for RedactingLine {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for RedactingLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let clean = kumikit_kernel::redact::redact(&text);
        // Nowhere to report a failed write to stderr.
        let _ = std::io::stderr().lock().write_all(clean.as_bytes());
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RedactingStderr {
    type Writer = RedactingLine;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingLine::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_buffered_until_dropped() {
        let mut line = RedactingLine::default();
        line.write_all(b"a ").unwrap();
        line.write_all(b"b\n").unwrap();
        assert_eq!(line.0, b"a b\n");
    }
}
