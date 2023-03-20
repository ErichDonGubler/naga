pub(crate) trait LogIfError<T> {
    fn log_if_err(self, found_err: &mut bool) -> Option<T>;
}

impl<T> LogIfError<T> for anyhow::Result<T> {
    fn log_if_err(self, found_err: &mut bool) -> Option<T> {
        match self {
            Ok(t) => Some(t),
            Err(e) => {
                log::error!("{e:?}");
                *found_err = true;
                None
            }
        }
    }
}
