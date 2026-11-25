use maxminddb::MaxMindDbError;
use napi::{
    bindgen_prelude::{Env, JsObjectValue, JsValue},
    Error, Status,
};
use std::io;

const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";

/// An error from opening a database file. File system failures keep the
/// details Node callers expect on `fs` errors (`code`, `syscall` and `path`);
/// everything else is an ordinary napi error.
pub(crate) enum OpenError {
    File(FileError),
    Other(Error),
}

pub(crate) type OpenResult<T> = std::result::Result<T, OpenError>;

impl From<Error> for OpenError {
    fn from(err: Error) -> Self {
        OpenError::Other(err)
    }
}

impl From<FileError> for OpenError {
    fn from(err: FileError) -> Self {
        OpenError::File(err)
    }
}

pub(crate) struct FileError {
    code: &'static str,
    description: String,
    syscall: &'static str,
    path: String,
}

impl FileError {
    pub(crate) fn from_io(err: &io::Error, syscall: &'static str, path: &str) -> Self {
        FileError {
            code: io_error_code(err),
            description: io_error_description(err),
            syscall,
            path: path.to_owned(),
        }
    }

    pub(crate) fn too_large(syscall: &'static str, path: &str) -> Self {
        FileError {
            code: "EFBIG",
            description: "file too large to map into memory".to_owned(),
            syscall,
            path: path.to_owned(),
        }
    }

    pub(crate) fn is_a_directory(syscall: &'static str, path: &str) -> Self {
        FileError {
            code: "EISDIR",
            description: "illegal operation on a directory".to_owned(),
            syscall,
            path: path.to_owned(),
        }
    }

    pub(crate) fn message(&self) -> String {
        format!(
            "{}: {}, {} '{}'",
            self.code, self.description, self.syscall, self.path
        )
    }

    /// Converts the error into a JS `Error` with `code`, `syscall` and `path`
    /// set, like Node's own `fs` errors, plus `readSequence` when the failed
    /// read has one. Must be called on the JS thread.
    pub(crate) fn into_napi(self, env: &Env, read_sequence: Option<u64>) -> Error {
        let message = self.message();
        let build = || -> napi::Result<Error> {
            let mut js_error = env.create_error(Error::new(Status::GenericFailure, &message))?;
            js_error.set_named_property("code", self.code)?;
            js_error.set_named_property("syscall", self.syscall)?;
            js_error.set_named_property("path", self.path.as_str())?;
            if let Some(read_sequence) = read_sequence {
                js_error.set_named_property("readSequence", read_sequence as f64)?;
            }
            Ok(Error::from(js_error.to_unknown()))
        };
        // If the JS object cannot be built, still keep the code and path in
        // the message.
        build().unwrap_or_else(|_| Error::new(Status::GenericFailure, message))
    }
}

/// Returns the OS description without Rust's " (os error N)" suffix, so the
/// message reads like a Node `fs` error.
fn io_error_description(err: &io::Error) -> String {
    let text = err.to_string();
    if let Some(raw) = err.raw_os_error() {
        let suffix = format!(" (os error {raw})");
        if let Some(stripped) = text.strip_suffix(&suffix) {
            return stripped.to_owned();
        }
    }
    text
}

/// Maps an I/O error to the Node-style error code (`ENOENT`, `EACCES`, ...).
fn io_error_code(err: &io::Error) -> &'static str {
    #[cfg(unix)]
    if let Some(code) = err.raw_os_error().and_then(errno_code) {
        return code;
    }
    #[cfg(windows)]
    if let Some(code) = err.raw_os_error().and_then(windows_error_code) {
        return code;
    }
    error_kind_code(err.kind())
}

#[cfg(unix)]
fn errno_code(errno: i32) -> Option<&'static str> {
    let code = match errno {
        libc::EACCES => "EACCES",
        libc::EAGAIN => "EAGAIN",
        libc::EBADF => "EBADF",
        libc::EBUSY => "EBUSY",
        libc::EDQUOT => "EDQUOT",
        libc::EFBIG => "EFBIG",
        libc::EINTR => "EINTR",
        libc::EINVAL => "EINVAL",
        libc::EIO => "EIO",
        libc::EISDIR => "EISDIR",
        libc::ELOOP => "ELOOP",
        libc::EMFILE => "EMFILE",
        libc::ENAMETOOLONG => "ENAMETOOLONG",
        libc::ENFILE => "ENFILE",
        libc::ENODEV => "ENODEV",
        libc::ENOENT => "ENOENT",
        libc::ENOMEM => "ENOMEM",
        libc::ENOSPC => "ENOSPC",
        libc::ENOTDIR => "ENOTDIR",
        libc::ENXIO => "ENXIO",
        libc::EOPNOTSUPP => "EOPNOTSUPP",
        libc::EOVERFLOW => "EOVERFLOW",
        libc::EPERM => "EPERM",
        libc::EROFS => "EROFS",
        libc::ESTALE => "ESTALE",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::ETXTBSY => "ETXTBSY",
        _ => return None,
    };
    Some(code)
}

/// Maps common Win32 file errors the same way libuv's `uv_translate_sys_error`
/// does, so the codes match what Node's own `fs` functions report on Windows.
/// Other errors fall back to the `io::ErrorKind` mapping.
#[cfg(windows)]
fn windows_error_code(error: i32) -> Option<&'static str> {
    let code = match error {
        // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_INVALID_DRIVE,
        // ERROR_INVALID_NAME, ERROR_BAD_PATHNAME, ERROR_DIRECTORY
        2 | 3 | 15 | 123 | 161 | 267 => "ENOENT",
        // ERROR_TOO_MANY_OPEN_FILES
        4 => "EMFILE",
        // ERROR_ACCESS_DENIED
        5 => "EPERM",
        // ERROR_NOT_ENOUGH_MEMORY, ERROR_OUTOFMEMORY
        8 | 14 => "ENOMEM",
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        32 | 33 => "EBUSY",
        // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
        39 | 112 => "ENOSPC",
        // ERROR_FILENAME_EXCED_RANGE
        206 => "ENAMETOOLONG",
        // ERROR_CANT_ACCESS_FILE
        1920 => "EACCES",
        _ => return None,
    };
    Some(code)
}

fn error_kind_code(kind: io::ErrorKind) -> &'static str {
    use io::ErrorKind;
    match kind {
        ErrorKind::NotFound => "ENOENT",
        ErrorKind::PermissionDenied => "EACCES",
        ErrorKind::IsADirectory => "EISDIR",
        ErrorKind::NotADirectory => "ENOTDIR",
        ErrorKind::ReadOnlyFilesystem => "EROFS",
        ErrorKind::ResourceBusy => "EBUSY",
        ErrorKind::ExecutableFileBusy => "ETXTBSY",
        ErrorKind::Interrupted => "EINTR",
        ErrorKind::OutOfMemory => "ENOMEM",
        ErrorKind::InvalidInput => "EINVAL",
        ErrorKind::WouldBlock => "EAGAIN",
        _ => "UNKNOWN",
    }
}

pub(crate) fn open_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::Io(io_err) => Error::new(Status::GenericFailure, io_err.to_string()),
        MaxMindDbError::InvalidDatabase { .. }
        | MaxMindDbError::Decoding { .. }
        | MaxMindDbError::ResourceLimit { .. } => Error::new(Status::GenericFailure, ERR_BAD_DATA),
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

pub(crate) fn lookup_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::InvalidDatabase { .. }
        | MaxMindDbError::Decoding { .. }
        | MaxMindDbError::ResourceLimit { .. } => Error::new(Status::GenericFailure, ERR_BAD_DATA),
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

pub(crate) fn invalid_arg(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

pub(crate) fn napi_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_not_found() {
        let err = io::Error::from(io::ErrorKind::NotFound);
        let file_err = FileError::from_io(&err, "open", "/nope.mmdb");
        assert_eq!(file_err.code, "ENOENT");
        assert!(file_err.message().starts_with("ENOENT: "));
        assert!(file_err.message().ends_with(", open '/nope.mmdb'"));
    }

    #[cfg(unix)]
    #[test]
    fn maps_raw_errno_and_strips_os_error_suffix() {
        let err = io::Error::from_raw_os_error(libc::ENOENT);
        let file_err = FileError::from_io(&err, "open", "/nope.mmdb");
        assert_eq!(file_err.code, "ENOENT");
        assert!(!file_err.description.contains("os error"));

        let err = io::Error::from_raw_os_error(libc::EMFILE);
        assert_eq!(FileError::from_io(&err, "open", "x").code, "EMFILE");

        let err = io::Error::from_raw_os_error(libc::EPERM);
        assert_eq!(FileError::from_io(&err, "open", "x").code, "EPERM");
    }

    #[cfg(windows)]
    #[test]
    fn maps_win32_errors_like_libuv() {
        assert_eq!(windows_error_code(2), Some("ENOENT"));
        assert_eq!(windows_error_code(5), Some("EPERM"));
        assert_eq!(windows_error_code(1920), Some("EACCES"));
        // libuv has no case for ERROR_NOT_READY.
        assert_eq!(windows_error_code(21), None);
    }

    #[test]
    fn unknown_kind_maps_to_unknown() {
        let err = io::Error::other("boom");
        let file_err = FileError::from_io(&err, "read", "x");
        assert_eq!(file_err.code, "UNKNOWN");
        assert_eq!(file_err.message(), "UNKNOWN: boom, read 'x'");
    }

    #[test]
    fn directory_error() {
        let file_err = FileError::is_a_directory("read", "/tmp");
        assert_eq!(
            file_err.message(),
            "EISDIR: illegal operation on a directory, read '/tmp'"
        );
    }
}
