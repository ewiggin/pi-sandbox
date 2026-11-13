use crate::Result;
use std::{
    error::Error,
    fmt, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct PathError {
    operation: &'static str,
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug formatting quotes paths and escapes control characters in names.
        write!(
            f,
            "cannot {} {:?}: {}",
            self.operation, self.path, self.source
        )
    }
}

impl Error for PathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

pub fn path_error(operation: &'static str, path: &Path, source: io::Error) -> Box<dyn Error> {
    Box::new(PathError {
        operation,
        path: path.to_owned(),
        source,
    })
}

pub trait PathContext<T> {
    fn at(self, operation: &'static str, path: &Path) -> Result<T>;
}

impl<T> PathContext<T> for io::Result<T> {
    fn at(self, operation: &'static str, path: &Path) -> Result<T> {
        self.map_err(|source| path_error(operation, path, source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_errors_preserve_operation_path_and_source() {
        let error = Err::<(), _>(io::Error::from_raw_os_error(libc::EACCES))
            .at("scan project directory", Path::new("/project/private"))
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("scan project directory"));
        assert!(message.contains("/project/private"));
        assert!(message.contains("Permission denied"));
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EACCES)
        );
    }
}
