use std::{ops::ControlFlow, time::Duration};

#[derive(Debug)]
pub enum AttemptError<E> {
    #[allow(dead_code)] // possible outcome, not currently produced
    Timeout,
    RetriesExhausted,
    CrticalError(E),
}

pub async fn attempt<F, G, R, E, S: ?Sized>(
    timeout: Option<Duration>,
    retry_count: usize,
    shared: &mut S,
    before_retry: F,
    f: G,
) -> Result<R, AttemptError<E>>
where
    F: AsyncFn(&mut S) -> Result<(), ()>,
    G: AsyncFn(&mut S) -> ControlFlow<Result<R, E>, ()>,
{
    tokio::pin!(f);
    let mut timeout = timeout.unwrap_or(Duration::from_secs(u64::MAX));
    for _ in 0..retry_count {
        match tokio::time::timeout(timeout, f(&mut *shared)).await {
            Ok(ControlFlow::Break(result)) => {
                return result.map_err(|e| AttemptError::CrticalError(e));
            }
            _ => {
                before_retry(&mut *shared)
                    .await
                    .map_err(|_| AttemptError::RetriesExhausted)?;
                timeout *= 2;
            }
        }
    }
    Err(AttemptError::RetriesExhausted)
}
