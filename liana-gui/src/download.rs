// This is based on https://github.com/iced-rs/iced/blob/master/examples/download_progress/src/download.rs
// with some modifications to store the downloaded bytes in `Progress::Finished` and `State::Downloading`
// and to keep track of any download errors.
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::stream::try_channel;
use iced::Subscription;

use std::hash::Hash;
use std::sync::Arc;

// Just a little utility function
pub fn file<I: 'static + Hash + Copy + Send + Sync, T: ToString>(
    id: I,
    url: T,
) -> iced::Subscription<(I, Result<Progress, DownloadError>)> {
    Subscription::run_with_id(
        id,
        download(url.to_string()).map(move |progress| (id, progress)),
    )
}

fn download(url: String) -> impl Stream<Item = Result<Progress, DownloadError>> {
    try_channel(100, move |mut output| async move {
        let response = reqwest::get(&url).await?;
        let total = response.content_length();

        let _ = output.send(Progress::Downloading(0.0)).await;

        let mut byte_stream = response.bytes_stream();
        let mut downloaded = 0;
        let mut bytes = Vec::new();

        while let Some(next_bytes) = byte_stream.next().await {
            let chunk = next_bytes?;
            downloaded += chunk.len();
            bytes.append(&mut chunk.to_vec());

            if let Some(total) = total {
                let _ = output
                    .send(Progress::Downloading(
                        100.0 * downloaded as f32 / total as f32,
                    ))
                    .await;
            }
        }

        let _ = output.send(Progress::Finished(bytes)).await;

        Ok(())
    })
}

#[derive(Debug, Clone)]
pub enum Progress {
    Downloading(f32),
    Finished(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum DownloadError {
    RequestFailed(Arc<reqwest::Error>),
    NoContentLength,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NoContentLength => {
                write!(f, "Response has unknown content length.")
            }
            Self::RequestFailed(e) => {
                write!(f, "Request error: '{}'.", e)
            }
        }
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(error: reqwest::Error) -> Self {
        DownloadError::RequestFailed(Arc::new(error))
    }
}

// The approach for tracking download progress is taken from
// https://github.com/iced-rs/iced/blob/master/examples/download_progress/src/main.rs.
#[derive(Debug)]
pub struct Download {
    id: usize,
    state: DownloadState,
}

impl Download {
    pub fn state(&self) -> &DownloadState {
        &self.state
    }

    pub fn finished_content(&self) -> Option<&[u8]> {
        if let DownloadState::Finished(bytes) = &self.state {
            Some(bytes)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum DownloadState {
    Idle,
    Downloading { progress: f32 },
    Finished(Vec<u8>),
    Errored(DownloadError),
}

impl Download {
    pub fn new(id: usize) -> Self {
        Download {
            id,
            state: DownloadState::Idle,
        }
    }

    pub fn start(&mut self) {
        match self.state {
            DownloadState::Idle { .. }
            | DownloadState::Finished { .. }
            | DownloadState::Errored { .. } => {
                self.state = DownloadState::Downloading { progress: 0.0 };
            }
            _ => {}
        }
    }

    pub fn progress(&mut self, new_progress: Result<Progress, DownloadError>) {
        if let DownloadState::Downloading { progress } = &mut self.state {
            match new_progress {
                Ok(Progress::Downloading(percentage)) => {
                    *progress = percentage;
                }
                Ok(Progress::Finished(bytes)) => {
                    self.state = DownloadState::Finished(bytes);
                }
                Err(e) => {
                    self.state = DownloadState::Errored(e);
                }
            }
        }
    }

    // pub fn subscription<U: ToString, M: Clone + Send + Sync>(&self, url: U, message: M) -> Subscription<M> {
    //     match self.state {
    //         DownloadState::Downloading { .. } => file(self.id, url)
    //             .map(|(_, progress)| {
    //                 message
    //             }),
    //         _ => Subscription::none(),
    //     }
    // }
}
