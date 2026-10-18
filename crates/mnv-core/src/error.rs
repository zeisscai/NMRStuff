use thiserror::Error;

#[derive(Debug, Error)]
pub enum MnvError {
    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("格式解析失败：{0}")]
    Parse(String),

    #[error("暂不支持的格式：{0}")]
    Unsupported(String),

    #[error("数据不完整：{0}")]
    Incomplete(String),

    #[error("处理失败：{0}")]
    Processing(String),

    #[error("序列化错误：{0}")]
    Serde(#[from] serde_json::Error),
}

impl MnvError {
    pub fn parse(msg: impl Into<String>) -> Self {
        MnvError::Parse(msg.into())
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        MnvError::Unsupported(msg.into())
    }

    pub fn incomplete(msg: impl Into<String>) -> Self {
        MnvError::Incomplete(msg.into())
    }

    pub fn processing(msg: impl Into<String>) -> Self {
        MnvError::Processing(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, MnvError>;
