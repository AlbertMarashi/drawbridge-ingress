
#[derive(Debug)]
pub enum IngressLoadBalancerError {
    General(Code, Box<str>),
    HyperError(hyper::Error),
}

#[derive(Debug)]
pub enum Code {
    NonExistentHost,
    CouldNotReachBackend,
    WebsocketUpgradeError,
    HttpError,
    InternalServerError,
    TLSError,
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Code::NonExistentHost => write!(f, "NonExistentHost"),
            Code::CouldNotReachBackend => write!(f, "CouldNotReachBackend"),
            Code::WebsocketUpgradeError => write!(f, "WebsocketUpgradeError"),
            Code::HttpError => write!(f, "HttpError"),
            Code::InternalServerError => write!(f, "InternalServerError"),
            Code::TLSError => write!(f, "TLSError"),
        }
    }
}

impl std::fmt::Display for IngressLoadBalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngressLoadBalancerError::General(code, msg) => write!(f, "Error: {}: {}", code, msg),
            IngressLoadBalancerError::HyperError(err) => write!(f, "Error: {}", err),
        }
    }
}

impl std::error::Error for IngressLoadBalancerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IngressLoadBalancerError::General(_, _) => None,
            IngressLoadBalancerError::HyperError(err) => Some(err),
        }
    }
}

impl IngressLoadBalancerError {
    pub fn general_as_result<M>(code: Code, msg: M) -> Result<!, Self>
    where
        M: Into<Box<str>>,
    {
        Err(Self::General(code, msg.into()))
    }

    pub fn general<M>(code: Code, msg: M) -> Self
    where
        M: Into<Box<str>>,
    {
        Self::General(code, msg.into())
    }
}