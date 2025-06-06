use alloc::vec;
use axerrno::AxResult;
use axio::{BufReader, prelude::*};
use axsync::Mutex;

#[cfg(feature = "fd")]
use {alloc::sync::Arc, axerrno::LinuxError, axerrno::LinuxResult, axio::PollState};

fn console_read_bytes(buf: &mut [u8]) -> AxResult<usize> {
    // we must make sure the buffer is in kernel memory
    let mut kernel_buf = vec![0u8; buf.len()];
    let len = axhal::console::read_bytes(&mut kernel_buf);
    buf.copy_from_slice(&kernel_buf);
    for c in &mut buf[..len] {
        if *c == b'\r' {
            *c = b'\n';
        }
    }
    Ok(len)
}

fn console_write_bytes(buf: &[u8]) -> AxResult<usize> {
    axhal::console::write_bytes(buf);
    Ok(buf.len())
}

struct StdinRaw;
struct StdoutRaw;

impl Read for StdinRaw {
    // Non-blocking read, returns number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> AxResult<usize> {
        let mut read_len = 0;
        while read_len < buf.len() {
            let len = console_read_bytes(buf[read_len..].as_mut())?;
            if len == 0 {
                break;
            }
            read_len += len;
        }
        Ok(read_len)
    }
}

impl Write for StdoutRaw {
    fn write(&mut self, buf: &[u8]) -> AxResult<usize> {
        console_write_bytes(buf)
    }

    fn flush(&mut self) -> AxResult {
        Ok(())
    }
}

#[derive(Default)]
struct StdinBuffer {
    buffer: [u8; 1],
    available: bool,
}

pub struct Stdin {
    inner: &'static Mutex<BufReader<StdinRaw>>,
    buffer: Mutex<StdinBuffer>,
}

impl Stdin {
    // Block until at least one byte is read.
    fn read_blocked(&self, buf: &mut [u8]) -> AxResult<usize> {
        // make sure buf[0] is valid
        if buf.is_empty() {
            return Ok(0);
        }
        let mut read_len = 0;
        let mut stdin_buffer = self.buffer.lock();
        let buf = if stdin_buffer.available {
            buf[0] = stdin_buffer.buffer[0];
            read_len += 1;
            stdin_buffer.available = false;
            &mut buf[1..]
        } else {
            buf
        };
        drop(stdin_buffer);
        read_len += self.inner.lock().read(buf)?;
        if read_len > 0 {
            return Ok(read_len);
        }
        // read_len == 0, try again until we get something
        loop {
            let read_len = self.inner.lock().read(buf)?;
            if read_len > 0 {
                return Ok(read_len);
            }
            crate::sys_sched_yield();
        }
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> AxResult<usize> {
        self.read_blocked(buf)
    }
}

pub struct Stdout {
    inner: &'static Mutex<StdoutRaw>,
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> AxResult<usize> {
        self.inner.lock().write(buf)
    }

    fn flush(&mut self) -> AxResult {
        self.inner.lock().flush()
    }
}

/// Constructs a new handle to the standard input of the current process.
pub fn stdin() -> Stdin {
    static INSTANCE: Mutex<BufReader<StdinRaw>> = Mutex::new(BufReader::new(StdinRaw));
    Stdin {
        inner: &INSTANCE,
        buffer: Default::default(),
    }
}

/// Constructs a new handle to the standard output of the current process.
pub fn stdout() -> Stdout {
    static INSTANCE: Mutex<StdoutRaw> = Mutex::new(StdoutRaw);
    Stdout { inner: &INSTANCE }
}

#[cfg(feature = "fd")]
impl super::fd_ops::FileLike for Stdin {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        Ok(self.read_blocked(buf)?)
    }

    fn write(&self, _buf: &[u8]) -> LinuxResult<usize> {
        Err(LinuxError::EPERM)
    }

    fn stat(&self) -> LinuxResult<crate::ctypes::stat> {
        let st_mode = 0o20000 | 0o440u32; // S_IFCHR | r--r-----
        Ok(crate::ctypes::stat {
            st_ino: 1,
            st_nlink: 1,
            st_mode,
            ..Default::default()
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        // try unblocking read
        let mut buf = [0u8; 1];
        let read_len = self.inner.lock().read(&mut buf)?;
        let readable = read_len > 0;
        if readable {
            // if we read something, we should store it in the buffer
            let mut stdin_buffer = self.buffer.lock();
            stdin_buffer.buffer[0] = buf[0];
            stdin_buffer.available = true;
        }
        Ok(PollState {
            readable,
            writable: true,
        })
    }

    fn set_nonblocking(&self, _nonblocking: bool) -> LinuxResult {
        Ok(())
    }
}

#[cfg(feature = "fd")]
impl super::fd_ops::FileLike for Stdout {
    fn read(&self, _buf: &mut [u8]) -> LinuxResult<usize> {
        Err(LinuxError::EPERM)
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        Ok(self.inner.lock().write(buf)?)
    }

    fn stat(&self) -> LinuxResult<crate::ctypes::stat> {
        let st_mode = 0o20000 | 0o220u32; // S_IFCHR | -w--w----
        Ok(crate::ctypes::stat {
            st_ino: 1,
            st_nlink: 1,
            st_mode,
            ..Default::default()
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn core::any::Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        Ok(PollState {
            readable: false,
            writable: true,
        })
    }

    fn set_nonblocking(&self, _nonblocking: bool) -> LinuxResult {
        Ok(())
    }
}
