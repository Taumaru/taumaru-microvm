pub(crate) mod human;

use crate::commands::download::DownloadProgressView;

pub(crate) trait ProgressSink {
    fn on_progress(&mut self, progress: DownloadProgressView);
}
