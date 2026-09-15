impl serde::Serialize for CallParams {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.context.is_some() {
            len += 1;
        }
        if self.timeout_budget_ms != 0 {
            len += 1;
        }
        if self.input.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.CallParams", len)?;
        if let Some(v) = self.context.as_ref() {
            struct_ser.serialize_field("context", v)?;
        }
        if self.timeout_budget_ms != 0 {
            struct_ser.serialize_field("timeoutBudgetMs", &self.timeout_budget_ms)?;
        }
        if let Some(v) = self.input.as_ref() {
            struct_ser.serialize_field("input", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for CallParams {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "context",
            "timeout_budget_ms",
            "timeoutBudgetMs",
            "input",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Context,
            TimeoutBudgetMs,
            Input,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "context" => Ok(GeneratedField::Context),
                            "timeoutBudgetMs" | "timeout_budget_ms" => Ok(GeneratedField::TimeoutBudgetMs),
                            "input" => Ok(GeneratedField::Input),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = CallParams;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.CallParams")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<CallParams, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut context__ = None;
                let mut timeout_budget_ms__ = None;
                let mut input__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Context => {
                            if context__.is_some() {
                                return Err(serde::de::Error::duplicate_field("context"));
                            }
                            context__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutBudgetMs => {
                            if timeout_budget_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutBudgetMs"));
                            }
                            timeout_budget_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Input => {
                            if input__.is_some() {
                                return Err(serde::de::Error::duplicate_field("input"));
                            }
                            input__ = map_.next_value()?;
                        }
                    }
                }
                Ok(CallParams {
                    context: context__,
                    timeout_budget_ms: timeout_budget_ms__.unwrap_or_default(),
                    input: input__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.CallParams", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Error {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.code != 0 {
            len += 1;
        }
        if !self.message.is_empty() {
            len += 1;
        }
        if !self.diagnostic_id.is_empty() {
            len += 1;
        }
        if !self.execution.is_empty() {
            len += 1;
        }
        if !self.retry.is_empty() {
            len += 1;
        }
        if self.details.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Error", len)?;
        if self.code != 0 {
            let v = ErrorCode::try_from(self.code)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.code)))?;
            struct_ser.serialize_field("code", &v)?;
        }
        if !self.message.is_empty() {
            struct_ser.serialize_field("message", &self.message)?;
        }
        if !self.diagnostic_id.is_empty() {
            struct_ser.serialize_field("diagnosticId", &self.diagnostic_id)?;
        }
        if !self.execution.is_empty() {
            struct_ser.serialize_field("execution", &self.execution)?;
        }
        if !self.retry.is_empty() {
            struct_ser.serialize_field("retry", &self.retry)?;
        }
        if let Some(v) = self.details.as_ref() {
            struct_ser.serialize_field("details", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Error {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "code",
            "message",
            "diagnostic_id",
            "diagnosticId",
            "execution",
            "retry",
            "details",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Code,
            Message,
            DiagnosticId,
            Execution,
            Retry,
            Details,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "code" => Ok(GeneratedField::Code),
                            "message" => Ok(GeneratedField::Message),
                            "diagnosticId" | "diagnostic_id" => Ok(GeneratedField::DiagnosticId),
                            "execution" => Ok(GeneratedField::Execution),
                            "retry" => Ok(GeneratedField::Retry),
                            "details" => Ok(GeneratedField::Details),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Error;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Error")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Error, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut code__ = None;
                let mut message__ = None;
                let mut diagnostic_id__ = None;
                let mut execution__ = None;
                let mut retry__ = None;
                let mut details__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Code => {
                            if code__.is_some() {
                                return Err(serde::de::Error::duplicate_field("code"));
                            }
                            code__ = Some(map_.next_value::<ErrorCode>()? as i32);
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DiagnosticId => {
                            if diagnostic_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("diagnosticId"));
                            }
                            diagnostic_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Execution => {
                            if execution__.is_some() {
                                return Err(serde::de::Error::duplicate_field("execution"));
                            }
                            execution__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Retry => {
                            if retry__.is_some() {
                                return Err(serde::de::Error::duplicate_field("retry"));
                            }
                            retry__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Details => {
                            if details__.is_some() {
                                return Err(serde::de::Error::duplicate_field("details"));
                            }
                            details__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Error {
                    code: code__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                    diagnostic_id: diagnostic_id__.unwrap_or_default(),
                    execution: execution__.unwrap_or_default(),
                    retry: retry__.unwrap_or_default(),
                    details: details__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Error", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ErrorCode {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "ERROR_CODE_UNSPECIFIED",
            Self::ParseError => "ERROR_CODE_PARSE_ERROR",
            Self::BadRequest => "ERROR_CODE_BAD_REQUEST",
            Self::UnknownMethod => "ERROR_CODE_UNKNOWN_METHOD",
            Self::Internal => "ERROR_CODE_INTERNAL",
            Self::Unauthorized => "ERROR_CODE_UNAUTHORIZED",
            Self::Forbidden => "ERROR_CODE_FORBIDDEN",
            Self::UnknownProvider => "ERROR_CODE_UNKNOWN_PROVIDER",
            Self::UnsupportedCapability => "ERROR_CODE_UNSUPPORTED_CAPABILITY",
            Self::Unavailable => "ERROR_CODE_UNAVAILABLE",
            Self::Timeout => "ERROR_CODE_TIMEOUT",
            Self::PayloadTooLarge => "ERROR_CODE_PAYLOAD_TOO_LARGE",
            Self::QuotaExceeded => "ERROR_CODE_QUOTA_EXCEEDED",
            Self::PlaneMismatch => "ERROR_CODE_PLANE_MISMATCH",
            Self::PeerUntrusted => "ERROR_CODE_PEER_UNTRUSTED",
            Self::Cancelled => "ERROR_CODE_CANCELLED",
            Self::OutcomeUnknown => "ERROR_CODE_OUTCOME_UNKNOWN",
            Self::StaleRevision => "ERROR_CODE_STALE_REVISION",
            Self::Conflict => "ERROR_CODE_CONFLICT",
            Self::SlowConsumer => "ERROR_CODE_SLOW_CONSUMER",
            Self::BadBlob => "ERROR_CODE_BAD_BLOB",
            Self::SessionLost => "ERROR_CODE_SESSION_LOST",
            Self::ResumeUnavailable => "ERROR_CODE_RESUME_UNAVAILABLE",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for ErrorCode {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ERROR_CODE_UNSPECIFIED",
            "ERROR_CODE_PARSE_ERROR",
            "ERROR_CODE_BAD_REQUEST",
            "ERROR_CODE_UNKNOWN_METHOD",
            "ERROR_CODE_INTERNAL",
            "ERROR_CODE_UNAUTHORIZED",
            "ERROR_CODE_FORBIDDEN",
            "ERROR_CODE_UNKNOWN_PROVIDER",
            "ERROR_CODE_UNSUPPORTED_CAPABILITY",
            "ERROR_CODE_UNAVAILABLE",
            "ERROR_CODE_TIMEOUT",
            "ERROR_CODE_PAYLOAD_TOO_LARGE",
            "ERROR_CODE_QUOTA_EXCEEDED",
            "ERROR_CODE_PLANE_MISMATCH",
            "ERROR_CODE_PEER_UNTRUSTED",
            "ERROR_CODE_CANCELLED",
            "ERROR_CODE_OUTCOME_UNKNOWN",
            "ERROR_CODE_STALE_REVISION",
            "ERROR_CODE_CONFLICT",
            "ERROR_CODE_SLOW_CONSUMER",
            "ERROR_CODE_BAD_BLOB",
            "ERROR_CODE_SESSION_LOST",
            "ERROR_CODE_RESUME_UNAVAILABLE",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = ErrorCode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "expected one of: {:?}", &FIELDS)
            }

            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), &self)
                    })
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), &self)
                    })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "ERROR_CODE_UNSPECIFIED" => Ok(ErrorCode::Unspecified),
                    "ERROR_CODE_PARSE_ERROR" => Ok(ErrorCode::ParseError),
                    "ERROR_CODE_BAD_REQUEST" => Ok(ErrorCode::BadRequest),
                    "ERROR_CODE_UNKNOWN_METHOD" => Ok(ErrorCode::UnknownMethod),
                    "ERROR_CODE_INTERNAL" => Ok(ErrorCode::Internal),
                    "ERROR_CODE_UNAUTHORIZED" => Ok(ErrorCode::Unauthorized),
                    "ERROR_CODE_FORBIDDEN" => Ok(ErrorCode::Forbidden),
                    "ERROR_CODE_UNKNOWN_PROVIDER" => Ok(ErrorCode::UnknownProvider),
                    "ERROR_CODE_UNSUPPORTED_CAPABILITY" => Ok(ErrorCode::UnsupportedCapability),
                    "ERROR_CODE_UNAVAILABLE" => Ok(ErrorCode::Unavailable),
                    "ERROR_CODE_TIMEOUT" => Ok(ErrorCode::Timeout),
                    "ERROR_CODE_PAYLOAD_TOO_LARGE" => Ok(ErrorCode::PayloadTooLarge),
                    "ERROR_CODE_QUOTA_EXCEEDED" => Ok(ErrorCode::QuotaExceeded),
                    "ERROR_CODE_PLANE_MISMATCH" => Ok(ErrorCode::PlaneMismatch),
                    "ERROR_CODE_PEER_UNTRUSTED" => Ok(ErrorCode::PeerUntrusted),
                    "ERROR_CODE_CANCELLED" => Ok(ErrorCode::Cancelled),
                    "ERROR_CODE_OUTCOME_UNKNOWN" => Ok(ErrorCode::OutcomeUnknown),
                    "ERROR_CODE_STALE_REVISION" => Ok(ErrorCode::StaleRevision),
                    "ERROR_CODE_CONFLICT" => Ok(ErrorCode::Conflict),
                    "ERROR_CODE_SLOW_CONSUMER" => Ok(ErrorCode::SlowConsumer),
                    "ERROR_CODE_BAD_BLOB" => Ok(ErrorCode::BadBlob),
                    "ERROR_CODE_SESSION_LOST" => Ok(ErrorCode::SessionLost),
                    "ERROR_CODE_RESUME_UNAVAILABLE" => Ok(ErrorCode::ResumeUnavailable),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for Failure {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.request_id.is_some() {
            len += 1;
        }
        if self.error.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Failure", len)?;
        if let Some(v) = self.request_id.as_ref() {
            struct_ser.serialize_field("requestId", v)?;
        }
        if let Some(v) = self.error.as_ref() {
            struct_ser.serialize_field("error", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Failure {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "request_id",
            "requestId",
            "error",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RequestId,
            Error,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "requestId" | "request_id" => Ok(GeneratedField::RequestId),
                            "error" => Ok(GeneratedField::Error),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Failure;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Failure")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Failure, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut request_id__ = None;
                let mut error__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RequestId => {
                            if request_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestId"));
                            }
                            request_id__ = map_.next_value()?;
                        }
                        GeneratedField::Error => {
                            if error__.is_some() {
                                return Err(serde::de::Error::duplicate_field("error"));
                            }
                            error__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Failure {
                    request_id: request_id__,
                    error: error__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Failure", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for HelloRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.profile_id.is_empty() {
            len += 1;
        }
        if self.plane != 0 {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.requires.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.HelloRequest", len)?;
        if !self.profile_id.is_empty() {
            struct_ser.serialize_field("profileId", &self.profile_id)?;
        }
        if self.plane != 0 {
            let v = Plane::try_from(self.plane)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.plane)))?;
            struct_ser.serialize_field("plane", &v)?;
        }
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if !self.requires.is_empty() {
            struct_ser.serialize_field("requires", &self.requires)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for HelloRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "profile_id",
            "profileId",
            "plane",
            "provides",
            "requires",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ProfileId,
            Plane,
            Provides,
            Requires,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "profileId" | "profile_id" => Ok(GeneratedField::ProfileId),
                            "plane" => Ok(GeneratedField::Plane),
                            "provides" => Ok(GeneratedField::Provides),
                            "requires" => Ok(GeneratedField::Requires),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = HelloRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.HelloRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<HelloRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut profile_id__ = None;
                let mut plane__ = None;
                let mut provides__ = None;
                let mut requires__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ProfileId => {
                            if profile_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("profileId"));
                            }
                            profile_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Plane => {
                            if plane__.is_some() {
                                return Err(serde::de::Error::duplicate_field("plane"));
                            }
                            plane__ = Some(map_.next_value::<Plane>()? as i32);
                        }
                        GeneratedField::Provides => {
                            if provides__.is_some() {
                                return Err(serde::de::Error::duplicate_field("provides"));
                            }
                            provides__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Requires => {
                            if requires__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requires"));
                            }
                            requires__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(HelloRequest {
                    profile_id: profile_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    requires: requires__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.HelloRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for HelloResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.binding_id.is_empty() {
            len += 1;
        }
        if self.expires_in_ms != 0 {
            len += 1;
        }
        if !self.profile_id.is_empty() {
            len += 1;
        }
        if self.plane != 0 {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.rejected_capabilities.is_empty() {
            len += 1;
        }
        if self.limits.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.HelloResponse", len)?;
        if !self.binding_id.is_empty() {
            struct_ser.serialize_field("bindingId", &self.binding_id)?;
        }
        if self.expires_in_ms != 0 {
            struct_ser.serialize_field("expiresInMs", &self.expires_in_ms)?;
        }
        if !self.profile_id.is_empty() {
            struct_ser.serialize_field("profileId", &self.profile_id)?;
        }
        if self.plane != 0 {
            let v = Plane::try_from(self.plane)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.plane)))?;
            struct_ser.serialize_field("plane", &v)?;
        }
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if !self.rejected_capabilities.is_empty() {
            struct_ser.serialize_field("rejectedCapabilities", &self.rejected_capabilities)?;
        }
        if let Some(v) = self.limits.as_ref() {
            struct_ser.serialize_field("limits", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for HelloResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "binding_id",
            "bindingId",
            "expires_in_ms",
            "expiresInMs",
            "profile_id",
            "profileId",
            "plane",
            "provides",
            "rejected_capabilities",
            "rejectedCapabilities",
            "limits",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            BindingId,
            ExpiresInMs,
            ProfileId,
            Plane,
            Provides,
            RejectedCapabilities,
            Limits,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "bindingId" | "binding_id" => Ok(GeneratedField::BindingId),
                            "expiresInMs" | "expires_in_ms" => Ok(GeneratedField::ExpiresInMs),
                            "profileId" | "profile_id" => Ok(GeneratedField::ProfileId),
                            "plane" => Ok(GeneratedField::Plane),
                            "provides" => Ok(GeneratedField::Provides),
                            "rejectedCapabilities" | "rejected_capabilities" => Ok(GeneratedField::RejectedCapabilities),
                            "limits" => Ok(GeneratedField::Limits),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = HelloResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.HelloResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<HelloResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut binding_id__ = None;
                let mut expires_in_ms__ = None;
                let mut profile_id__ = None;
                let mut plane__ = None;
                let mut provides__ = None;
                let mut rejected_capabilities__ = None;
                let mut limits__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::BindingId => {
                            if binding_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("bindingId"));
                            }
                            binding_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpiresInMs => {
                            if expires_in_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expiresInMs"));
                            }
                            expires_in_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ProfileId => {
                            if profile_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("profileId"));
                            }
                            profile_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Plane => {
                            if plane__.is_some() {
                                return Err(serde::de::Error::duplicate_field("plane"));
                            }
                            plane__ = Some(map_.next_value::<Plane>()? as i32);
                        }
                        GeneratedField::Provides => {
                            if provides__.is_some() {
                                return Err(serde::de::Error::duplicate_field("provides"));
                            }
                            provides__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RejectedCapabilities => {
                            if rejected_capabilities__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rejectedCapabilities"));
                            }
                            rejected_capabilities__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Limits => {
                            if limits__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limits"));
                            }
                            limits__ = map_.next_value()?;
                        }
                    }
                }
                Ok(HelloResponse {
                    binding_id: binding_id__.unwrap_or_default(),
                    expires_in_ms: expires_in_ms__.unwrap_or_default(),
                    profile_id: profile_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    rejected_capabilities: rejected_capabilities__.unwrap_or_default(),
                    limits: limits__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.HelloResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Limits {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.max_frame_bytes != 0 {
            len += 1;
        }
        if self.max_inflight != 0 {
            len += 1;
        }
        if self.max_queued_bytes != 0 {
            len += 1;
        }
        if self.timeout_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Limits", len)?;
        if self.max_frame_bytes != 0 {
            struct_ser.serialize_field("maxFrameBytes", &self.max_frame_bytes)?;
        }
        if self.max_inflight != 0 {
            struct_ser.serialize_field("maxInflight", &self.max_inflight)?;
        }
        if self.max_queued_bytes != 0 {
            struct_ser.serialize_field("maxQueuedBytes", &self.max_queued_bytes)?;
        }
        if self.timeout_ms != 0 {
            struct_ser.serialize_field("timeoutMs", &self.timeout_ms)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Limits {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "max_frame_bytes",
            "maxFrameBytes",
            "max_inflight",
            "maxInflight",
            "max_queued_bytes",
            "maxQueuedBytes",
            "timeout_ms",
            "timeoutMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MaxFrameBytes,
            MaxInflight,
            MaxQueuedBytes,
            TimeoutMs,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "maxFrameBytes" | "max_frame_bytes" => Ok(GeneratedField::MaxFrameBytes),
                            "maxInflight" | "max_inflight" => Ok(GeneratedField::MaxInflight),
                            "maxQueuedBytes" | "max_queued_bytes" => Ok(GeneratedField::MaxQueuedBytes),
                            "timeoutMs" | "timeout_ms" => Ok(GeneratedField::TimeoutMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Limits;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Limits")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Limits, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut max_frame_bytes__ = None;
                let mut max_inflight__ = None;
                let mut max_queued_bytes__ = None;
                let mut timeout_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MaxFrameBytes => {
                            if max_frame_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxFrameBytes"));
                            }
                            max_frame_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MaxInflight => {
                            if max_inflight__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxInflight"));
                            }
                            max_inflight__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MaxQueuedBytes => {
                            if max_queued_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxQueuedBytes"));
                            }
                            max_queued_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::TimeoutMs => {
                            if timeout_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutMs"));
                            }
                            timeout_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(Limits {
                    max_frame_bytes: max_frame_bytes__.unwrap_or_default(),
                    max_inflight: max_inflight__.unwrap_or_default(),
                    max_queued_bytes: max_queued_bytes__.unwrap_or_default(),
                    timeout_ms: timeout_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Limits", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Manifest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.manifest_version != 0 {
            len += 1;
        }
        if !self.id.is_empty() {
            len += 1;
        }
        if !self.endpoint_id.is_empty() {
            len += 1;
        }
        if !self.kind.is_empty() {
            len += 1;
        }
        if !self.display.is_empty() {
            len += 1;
        }
        if self.plane != 0 {
            len += 1;
        }
        if !self.transport.is_empty() {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.requires.is_empty() {
            len += 1;
        }
        if !self.profile_ids.is_empty() {
            len += 1;
        }
        if self.limits.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Manifest", len)?;
        if self.manifest_version != 0 {
            struct_ser.serialize_field("manifestVersion", &self.manifest_version)?;
        }
        if !self.id.is_empty() {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if !self.endpoint_id.is_empty() {
            struct_ser.serialize_field("endpointId", &self.endpoint_id)?;
        }
        if !self.kind.is_empty() {
            struct_ser.serialize_field("kind", &self.kind)?;
        }
        if !self.display.is_empty() {
            struct_ser.serialize_field("display", &self.display)?;
        }
        if self.plane != 0 {
            let v = Plane::try_from(self.plane)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.plane)))?;
            struct_ser.serialize_field("plane", &v)?;
        }
        if !self.transport.is_empty() {
            struct_ser.serialize_field("transport", &self.transport)?;
        }
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if !self.requires.is_empty() {
            struct_ser.serialize_field("requires", &self.requires)?;
        }
        if !self.profile_ids.is_empty() {
            struct_ser.serialize_field("profileIds", &self.profile_ids)?;
        }
        if let Some(v) = self.limits.as_ref() {
            struct_ser.serialize_field("limits", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Manifest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "manifest_version",
            "manifestVersion",
            "id",
            "endpoint_id",
            "endpointId",
            "kind",
            "display",
            "plane",
            "transport",
            "provides",
            "requires",
            "profile_ids",
            "profileIds",
            "limits",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ManifestVersion,
            Id,
            EndpointId,
            Kind,
            Display,
            Plane,
            Transport,
            Provides,
            Requires,
            ProfileIds,
            Limits,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "manifestVersion" | "manifest_version" => Ok(GeneratedField::ManifestVersion),
                            "id" => Ok(GeneratedField::Id),
                            "endpointId" | "endpoint_id" => Ok(GeneratedField::EndpointId),
                            "kind" => Ok(GeneratedField::Kind),
                            "display" => Ok(GeneratedField::Display),
                            "plane" => Ok(GeneratedField::Plane),
                            "transport" => Ok(GeneratedField::Transport),
                            "provides" => Ok(GeneratedField::Provides),
                            "requires" => Ok(GeneratedField::Requires),
                            "profileIds" | "profile_ids" => Ok(GeneratedField::ProfileIds),
                            "limits" => Ok(GeneratedField::Limits),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Manifest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Manifest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Manifest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut manifest_version__ = None;
                let mut id__ = None;
                let mut endpoint_id__ = None;
                let mut kind__ = None;
                let mut display__ = None;
                let mut plane__ = None;
                let mut transport__ = None;
                let mut provides__ = None;
                let mut requires__ = None;
                let mut profile_ids__ = None;
                let mut limits__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ManifestVersion => {
                            if manifest_version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("manifestVersion"));
                            }
                            manifest_version__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::EndpointId => {
                            if endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointId"));
                            }
                            endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Kind => {
                            if kind__.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Display => {
                            if display__.is_some() {
                                return Err(serde::de::Error::duplicate_field("display"));
                            }
                            display__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Plane => {
                            if plane__.is_some() {
                                return Err(serde::de::Error::duplicate_field("plane"));
                            }
                            plane__ = Some(map_.next_value::<Plane>()? as i32);
                        }
                        GeneratedField::Transport => {
                            if transport__.is_some() {
                                return Err(serde::de::Error::duplicate_field("transport"));
                            }
                            transport__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Provides => {
                            if provides__.is_some() {
                                return Err(serde::de::Error::duplicate_field("provides"));
                            }
                            provides__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Requires => {
                            if requires__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requires"));
                            }
                            requires__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ProfileIds => {
                            if profile_ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("profileIds"));
                            }
                            profile_ids__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Limits => {
                            if limits__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limits"));
                            }
                            limits__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Manifest {
                    manifest_version: manifest_version__.unwrap_or_default(),
                    id: id__.unwrap_or_default(),
                    endpoint_id: endpoint_id__.unwrap_or_default(),
                    kind: kind__.unwrap_or_default(),
                    display: display__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    transport: transport__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    requires: requires__.unwrap_or_default(),
                    profile_ids: profile_ids__.unwrap_or_default(),
                    limits: limits__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Manifest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Message {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.body.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Message", len)?;
        if let Some(v) = self.body.as_ref() {
            match v {
                message::Body::Request(v) => {
                    struct_ser.serialize_field("request", v)?;
                }
                message::Body::Success(v) => {
                    struct_ser.serialize_field("success", v)?;
                }
                message::Body::Failure(v) => {
                    struct_ser.serialize_field("failure", v)?;
                }
                message::Body::Notification(v) => {
                    struct_ser.serialize_field("notification", v)?;
                }
            }
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Message {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "request",
            "success",
            "failure",
            "notification",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Request,
            Success,
            Failure,
            Notification,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "request" => Ok(GeneratedField::Request),
                            "success" => Ok(GeneratedField::Success),
                            "failure" => Ok(GeneratedField::Failure),
                            "notification" => Ok(GeneratedField::Notification),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Message;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Message")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Message, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut body__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Request => {
                            if body__.is_some() {
                                return Err(serde::de::Error::duplicate_field("request"));
                            }
                            body__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Body::Request)
;
                        }
                        GeneratedField::Success => {
                            if body__.is_some() {
                                return Err(serde::de::Error::duplicate_field("success"));
                            }
                            body__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Body::Success)
;
                        }
                        GeneratedField::Failure => {
                            if body__.is_some() {
                                return Err(serde::de::Error::duplicate_field("failure"));
                            }
                            body__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Body::Failure)
;
                        }
                        GeneratedField::Notification => {
                            if body__.is_some() {
                                return Err(serde::de::Error::duplicate_field("notification"));
                            }
                            body__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Body::Notification)
;
                        }
                    }
                }
                Ok(Message {
                    body: body__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Message", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Notification {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.method.is_empty() {
            len += 1;
        }
        if self.params.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Notification", len)?;
        if !self.method.is_empty() {
            struct_ser.serialize_field("method", &self.method)?;
        }
        if let Some(v) = self.params.as_ref() {
            struct_ser.serialize_field("params", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Notification {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "method",
            "params",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Method,
            Params,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "method" => Ok(GeneratedField::Method),
                            "params" => Ok(GeneratedField::Params),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Notification;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Notification")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Notification, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut method__ = None;
                let mut params__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Method => {
                            if method__.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Params => {
                            if params__.is_some() {
                                return Err(serde::de::Error::duplicate_field("params"));
                            }
                            params__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Notification {
                    method: method__.unwrap_or_default(),
                    params: params__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Notification", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Plane {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "PLANE_UNSPECIFIED",
            Self::Broker => "PLANE_BROKER",
            Self::Relay => "PLANE_RELAY",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for Plane {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "PLANE_UNSPECIFIED",
            "PLANE_BROKER",
            "PLANE_RELAY",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = Plane;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "expected one of: {:?}", &FIELDS)
            }

            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), &self)
                    })
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), &self)
                    })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "PLANE_UNSPECIFIED" => Ok(Plane::Unspecified),
                    "PLANE_BROKER" => Ok(Plane::Broker),
                    "PLANE_RELAY" => Ok(Plane::Relay),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for RejectedCapability {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.method.is_empty() {
            len += 1;
        }
        if !self.direction.is_empty() {
            len += 1;
        }
        if !self.reason.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.RejectedCapability", len)?;
        if !self.method.is_empty() {
            struct_ser.serialize_field("method", &self.method)?;
        }
        if !self.direction.is_empty() {
            struct_ser.serialize_field("direction", &self.direction)?;
        }
        if !self.reason.is_empty() {
            struct_ser.serialize_field("reason", &self.reason)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RejectedCapability {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "method",
            "direction",
            "reason",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Method,
            Direction,
            Reason,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "method" => Ok(GeneratedField::Method),
                            "direction" => Ok(GeneratedField::Direction),
                            "reason" => Ok(GeneratedField::Reason),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RejectedCapability;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.RejectedCapability")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RejectedCapability, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut method__ = None;
                let mut direction__ = None;
                let mut reason__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Method => {
                            if method__.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Direction => {
                            if direction__.is_some() {
                                return Err(serde::de::Error::duplicate_field("direction"));
                            }
                            direction__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(RejectedCapability {
                    method: method__.unwrap_or_default(),
                    direction: direction__.unwrap_or_default(),
                    reason: reason__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.RejectedCapability", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Request {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.request_id.is_empty() {
            len += 1;
        }
        if !self.method.is_empty() {
            len += 1;
        }
        if self.params.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Request", len)?;
        if !self.request_id.is_empty() {
            struct_ser.serialize_field("requestId", &self.request_id)?;
        }
        if !self.method.is_empty() {
            struct_ser.serialize_field("method", &self.method)?;
        }
        if let Some(v) = self.params.as_ref() {
            struct_ser.serialize_field("params", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Request {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "request_id",
            "requestId",
            "method",
            "params",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RequestId,
            Method,
            Params,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "requestId" | "request_id" => Ok(GeneratedField::RequestId),
                            "method" => Ok(GeneratedField::Method),
                            "params" => Ok(GeneratedField::Params),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Request;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Request")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Request, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut request_id__ = None;
                let mut method__ = None;
                let mut params__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RequestId => {
                            if request_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestId"));
                            }
                            request_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Method => {
                            if method__.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Params => {
                            if params__.is_some() {
                                return Err(serde::de::Error::duplicate_field("params"));
                            }
                            params__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Request {
                    request_id: request_id__.unwrap_or_default(),
                    method: method__.unwrap_or_default(),
                    params: params__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Request", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for RequestContext {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.provider_endpoint_id.is_empty() {
            len += 1;
        }
        if self.plane != 0 {
            len += 1;
        }
        if self.binding_id.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.RequestContext", len)?;
        if !self.provider_endpoint_id.is_empty() {
            struct_ser.serialize_field("providerEndpointId", &self.provider_endpoint_id)?;
        }
        if self.plane != 0 {
            let v = Plane::try_from(self.plane)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.plane)))?;
            struct_ser.serialize_field("plane", &v)?;
        }
        if let Some(v) = self.binding_id.as_ref() {
            struct_ser.serialize_field("bindingId", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RequestContext {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "provider_endpoint_id",
            "providerEndpointId",
            "plane",
            "binding_id",
            "bindingId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ProviderEndpointId,
            Plane,
            BindingId,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "providerEndpointId" | "provider_endpoint_id" => Ok(GeneratedField::ProviderEndpointId),
                            "plane" => Ok(GeneratedField::Plane),
                            "bindingId" | "binding_id" => Ok(GeneratedField::BindingId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RequestContext;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.RequestContext")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RequestContext, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut provider_endpoint_id__ = None;
                let mut plane__ = None;
                let mut binding_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ProviderEndpointId => {
                            if provider_endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("providerEndpointId"));
                            }
                            provider_endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Plane => {
                            if plane__.is_some() {
                                return Err(serde::de::Error::duplicate_field("plane"));
                            }
                            plane__ = Some(map_.next_value::<Plane>()? as i32);
                        }
                        GeneratedField::BindingId => {
                            if binding_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("bindingId"));
                            }
                            binding_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(RequestContext {
                    provider_endpoint_id: provider_endpoint_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    binding_id: binding_id__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.RequestContext", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ResourceSummary {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.resource_id.is_empty() {
            len += 1;
        }
        if !self.title.is_empty() {
            len += 1;
        }
        if !self.mime.is_empty() {
            len += 1;
        }
        if self.size_bytes.is_some() {
            len += 1;
        }
        if self.revision.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.ResourceSummary", len)?;
        if !self.resource_id.is_empty() {
            struct_ser.serialize_field("resourceId", &self.resource_id)?;
        }
        if !self.title.is_empty() {
            struct_ser.serialize_field("title", &self.title)?;
        }
        if !self.mime.is_empty() {
            struct_ser.serialize_field("mime", &self.mime)?;
        }
        if let Some(v) = self.size_bytes.as_ref() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("sizeBytes", ToString::to_string(&v).as_str())?;
        }
        if let Some(v) = self.revision.as_ref() {
            struct_ser.serialize_field("revision", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ResourceSummary {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "resource_id",
            "resourceId",
            "title",
            "mime",
            "size_bytes",
            "sizeBytes",
            "revision",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ResourceId,
            Title,
            Mime,
            SizeBytes,
            Revision,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "resourceId" | "resource_id" => Ok(GeneratedField::ResourceId),
                            "title" => Ok(GeneratedField::Title),
                            "mime" => Ok(GeneratedField::Mime),
                            "sizeBytes" | "size_bytes" => Ok(GeneratedField::SizeBytes),
                            "revision" => Ok(GeneratedField::Revision),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ResourceSummary;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.ResourceSummary")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ResourceSummary, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource_id__ = None;
                let mut title__ = None;
                let mut mime__ = None;
                let mut size_bytes__ = None;
                let mut revision__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ResourceId => {
                            if resource_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceId"));
                            }
                            resource_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Title => {
                            if title__.is_some() {
                                return Err(serde::de::Error::duplicate_field("title"));
                            }
                            title__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Mime => {
                            if mime__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mime"));
                            }
                            mime__ = Some(map_.next_value()?);
                        }
                        GeneratedField::SizeBytes => {
                            if size_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sizeBytes"));
                            }
                            size_bytes__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ResourceSummary {
                    resource_id: resource_id__.unwrap_or_default(),
                    title: title__.unwrap_or_default(),
                    mime: mime__.unwrap_or_default(),
                    size_bytes: size_bytes__,
                    revision: revision__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.ResourceSummary", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SearchHit {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.resource.is_some() {
            len += 1;
        }
        if !self.excerpt.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SearchHit", len)?;
        if let Some(v) = self.resource.as_ref() {
            struct_ser.serialize_field("resource", v)?;
        }
        if !self.excerpt.is_empty() {
            struct_ser.serialize_field("excerpt", &self.excerpt)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SearchHit {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "resource",
            "excerpt",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Resource,
            Excerpt,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "resource" => Ok(GeneratedField::Resource),
                            "excerpt" => Ok(GeneratedField::Excerpt),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SearchHit;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SearchHit")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SearchHit, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource__ = None;
                let mut excerpt__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Resource => {
                            if resource__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resource"));
                            }
                            resource__ = map_.next_value()?;
                        }
                        GeneratedField::Excerpt => {
                            if excerpt__.is_some() {
                                return Err(serde::de::Error::duplicate_field("excerpt"));
                            }
                            excerpt__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(SearchHit {
                    resource: resource__,
                    excerpt: excerpt__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SearchHit", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceListRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.root.is_empty() {
            len += 1;
        }
        if self.limit.is_some() {
            len += 1;
        }
        if self.cursor.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceListRequest", len)?;
        if !self.root.is_empty() {
            struct_ser.serialize_field("root", &self.root)?;
        }
        if let Some(v) = self.limit.as_ref() {
            struct_ser.serialize_field("limit", v)?;
        }
        if let Some(v) = self.cursor.as_ref() {
            struct_ser.serialize_field("cursor", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceListRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "root",
            "limit",
            "cursor",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Root,
            Limit,
            Cursor,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "root" => Ok(GeneratedField::Root),
                            "limit" => Ok(GeneratedField::Limit),
                            "cursor" => Ok(GeneratedField::Cursor),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceListRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceListRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceListRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut root__ = None;
                let mut limit__ = None;
                let mut cursor__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Limit => {
                            if limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limit"));
                            }
                            limit__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::Cursor => {
                            if cursor__.is_some() {
                                return Err(serde::de::Error::duplicate_field("cursor"));
                            }
                            cursor__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SourceListRequest {
                    root: root__.unwrap_or_default(),
                    limit: limit__,
                    cursor: cursor__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceListRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceListResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.items.is_empty() {
            len += 1;
        }
        if self.next_cursor.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceListResponse", len)?;
        if !self.items.is_empty() {
            struct_ser.serialize_field("items", &self.items)?;
        }
        if let Some(v) = self.next_cursor.as_ref() {
            struct_ser.serialize_field("nextCursor", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceListResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "items",
            "next_cursor",
            "nextCursor",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Items,
            NextCursor,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "items" => Ok(GeneratedField::Items),
                            "nextCursor" | "next_cursor" => Ok(GeneratedField::NextCursor),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceListResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceListResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceListResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut items__ = None;
                let mut next_cursor__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Items => {
                            if items__.is_some() {
                                return Err(serde::de::Error::duplicate_field("items"));
                            }
                            items__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NextCursor => {
                            if next_cursor__.is_some() {
                                return Err(serde::de::Error::duplicate_field("nextCursor"));
                            }
                            next_cursor__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SourceListResponse {
                    items: items__.unwrap_or_default(),
                    next_cursor: next_cursor__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceListResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceReadRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.resource_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceReadRequest", len)?;
        if !self.resource_id.is_empty() {
            struct_ser.serialize_field("resourceId", &self.resource_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceReadRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "resource_id",
            "resourceId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ResourceId,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "resourceId" | "resource_id" => Ok(GeneratedField::ResourceId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceReadRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceReadRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceReadRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ResourceId => {
                            if resource_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceId"));
                            }
                            resource_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(SourceReadRequest {
                    resource_id: resource_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceReadRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceReadResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.resource.is_some() {
            len += 1;
        }
        if !self.text.is_empty() {
            len += 1;
        }
        if !self.cid.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceReadResponse", len)?;
        if let Some(v) = self.resource.as_ref() {
            struct_ser.serialize_field("resource", v)?;
        }
        if !self.text.is_empty() {
            struct_ser.serialize_field("text", &self.text)?;
        }
        if !self.cid.is_empty() {
            struct_ser.serialize_field("cid", &self.cid)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceReadResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "resource",
            "text",
            "cid",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Resource,
            Text,
            Cid,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "resource" => Ok(GeneratedField::Resource),
                            "text" => Ok(GeneratedField::Text),
                            "cid" => Ok(GeneratedField::Cid),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceReadResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceReadResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceReadResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource__ = None;
                let mut text__ = None;
                let mut cid__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Resource => {
                            if resource__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resource"));
                            }
                            resource__ = map_.next_value()?;
                        }
                        GeneratedField::Text => {
                            if text__.is_some() {
                                return Err(serde::de::Error::duplicate_field("text"));
                            }
                            text__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Cid => {
                            if cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("cid"));
                            }
                            cid__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(SourceReadResponse {
                    resource: resource__,
                    text: text__.unwrap_or_default(),
                    cid: cid__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceReadResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceSearchRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.root.is_empty() {
            len += 1;
        }
        if !self.query.is_empty() {
            len += 1;
        }
        if self.limit.is_some() {
            len += 1;
        }
        if self.cursor.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceSearchRequest", len)?;
        if !self.root.is_empty() {
            struct_ser.serialize_field("root", &self.root)?;
        }
        if !self.query.is_empty() {
            struct_ser.serialize_field("query", &self.query)?;
        }
        if let Some(v) = self.limit.as_ref() {
            struct_ser.serialize_field("limit", v)?;
        }
        if let Some(v) = self.cursor.as_ref() {
            struct_ser.serialize_field("cursor", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceSearchRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "root",
            "query",
            "limit",
            "cursor",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Root,
            Query,
            Limit,
            Cursor,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "root" => Ok(GeneratedField::Root),
                            "query" => Ok(GeneratedField::Query),
                            "limit" => Ok(GeneratedField::Limit),
                            "cursor" => Ok(GeneratedField::Cursor),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceSearchRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceSearchRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceSearchRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut root__ = None;
                let mut query__ = None;
                let mut limit__ = None;
                let mut cursor__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Query => {
                            if query__.is_some() {
                                return Err(serde::de::Error::duplicate_field("query"));
                            }
                            query__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Limit => {
                            if limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limit"));
                            }
                            limit__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::Cursor => {
                            if cursor__.is_some() {
                                return Err(serde::de::Error::duplicate_field("cursor"));
                            }
                            cursor__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SourceSearchRequest {
                    root: root__.unwrap_or_default(),
                    query: query__.unwrap_or_default(),
                    limit: limit__,
                    cursor: cursor__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceSearchRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SourceSearchResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.items.is_empty() {
            len += 1;
        }
        if self.next_cursor.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.SourceSearchResponse", len)?;
        if !self.items.is_empty() {
            struct_ser.serialize_field("items", &self.items)?;
        }
        if let Some(v) = self.next_cursor.as_ref() {
            struct_ser.serialize_field("nextCursor", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SourceSearchResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "items",
            "next_cursor",
            "nextCursor",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Items,
            NextCursor,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "items" => Ok(GeneratedField::Items),
                            "nextCursor" | "next_cursor" => Ok(GeneratedField::NextCursor),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SourceSearchResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.SourceSearchResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceSearchResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut items__ = None;
                let mut next_cursor__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Items => {
                            if items__.is_some() {
                                return Err(serde::de::Error::duplicate_field("items"));
                            }
                            items__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NextCursor => {
                            if next_cursor__.is_some() {
                                return Err(serde::de::Error::duplicate_field("nextCursor"));
                            }
                            next_cursor__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SourceSearchResponse {
                    items: items__.unwrap_or_default(),
                    next_cursor: next_cursor__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.SourceSearchResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Success {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.request_id.is_empty() {
            len += 1;
        }
        if self.result.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.v1.Success", len)?;
        if !self.request_id.is_empty() {
            struct_ser.serialize_field("requestId", &self.request_id)?;
        }
        if let Some(v) = self.result.as_ref() {
            struct_ser.serialize_field("result", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Success {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "request_id",
            "requestId",
            "result",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RequestId,
            Result,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl serde::de::Visitor<'_> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "requestId" | "request_id" => Ok(GeneratedField::RequestId),
                            "result" => Ok(GeneratedField::Result),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Success;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.v1.Success")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Success, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut request_id__ = None;
                let mut result__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RequestId => {
                            if request_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestId"));
                            }
                            request_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Result => {
                            if result__.is_some() {
                                return Err(serde::de::Error::duplicate_field("result"));
                            }
                            result__ = map_.next_value()?;
                        }
                    }
                }
                Ok(Success {
                    request_id: request_id__.unwrap_or_default(),
                    result: result__,
                })
            }
        }
        deserializer.deserialize_struct("conex.v1.Success", FIELDS, GeneratedVisitor)
    }
}
