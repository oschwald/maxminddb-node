use maxminddb::MaxMindDbError;
use napi::{Error, Status};

const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";

pub(crate) fn open_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::Io(io_err) => Error::new(Status::GenericFailure, io_err.to_string()),
        MaxMindDbError::InvalidDatabase { .. } | MaxMindDbError::Decoding { .. } => {
            Error::new(Status::GenericFailure, ERR_BAD_DATA)
        }
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

pub(crate) fn lookup_error(err: MaxMindDbError) -> Error {
    match err {
        MaxMindDbError::InvalidDatabase { .. } | MaxMindDbError::Decoding { .. } => {
            Error::new(Status::GenericFailure, ERR_BAD_DATA)
        }
        other => Error::new(Status::GenericFailure, other.to_string()),
    }
}

pub(crate) fn invalid_arg(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

pub(crate) fn napi_error(message: impl Into<String>) -> Error {
    Error::new(Status::GenericFailure, message.into())
}
