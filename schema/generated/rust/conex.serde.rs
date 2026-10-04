impl serde::Serialize for AgentEndpoint {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.endpoint_id.is_empty() {
            len += 1;
        }
        if !self.root.is_empty() {
            len += 1;
        }
        if !self.methods.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.AgentEndpoint", len)?;
        if !self.endpoint_id.is_empty() {
            struct_ser.serialize_field("endpointId", &self.endpoint_id)?;
        }
        if !self.root.is_empty() {
            struct_ser.serialize_field("root", &self.root)?;
        }
        if !self.methods.is_empty() {
            struct_ser.serialize_field("methods", &self.methods)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AgentEndpoint {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "endpoint_id",
            "endpointId",
            "root",
            "methods",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EndpointId,
            Root,
            Methods,
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
                            "endpointId" | "endpoint_id" => Ok(GeneratedField::EndpointId),
                            "root" => Ok(GeneratedField::Root),
                            "methods" => Ok(GeneratedField::Methods),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AgentEndpoint;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.AgentEndpoint")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AgentEndpoint, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut endpoint_id__ = None;
                let mut root__ = None;
                let mut methods__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EndpointId => {
                            if endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointId"));
                            }
                            endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Methods => {
                            if methods__.is_some() {
                                return Err(serde::de::Error::duplicate_field("methods"));
                            }
                            methods__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(AgentEndpoint {
                    endpoint_id: endpoint_id__.unwrap_or_default(),
                    root: root__.unwrap_or_default(),
                    methods: methods__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.AgentEndpoint", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for AgentLinkSummary {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.agent_id.is_empty() {
            len += 1;
        }
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if self.generation != 0 {
            len += 1;
        }
        if self.registered_at_ms != 0 {
            len += 1;
        }
        if self.last_heartbeat_at_ms != 0 {
            len += 1;
        }
        if !self.endpoint_ids.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.AgentLinkSummary", len)?;
        if !self.agent_id.is_empty() {
            struct_ser.serialize_field("agentId", &self.agent_id)?;
        }
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if self.generation != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("generation", ToString::to_string(&self.generation).as_str())?;
        }
        if self.registered_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("registeredAtMs", ToString::to_string(&self.registered_at_ms).as_str())?;
        }
        if self.last_heartbeat_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastHeartbeatAtMs", ToString::to_string(&self.last_heartbeat_at_ms).as_str())?;
        }
        if !self.endpoint_ids.is_empty() {
            struct_ser.serialize_field("endpointIds", &self.endpoint_ids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AgentLinkSummary {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "agent_id",
            "agentId",
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
            "generation",
            "registered_at_ms",
            "registeredAtMs",
            "last_heartbeat_at_ms",
            "lastHeartbeatAtMs",
            "endpoint_ids",
            "endpointIds",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            AgentId,
            PrincipalId,
            TenantId,
            Generation,
            RegisteredAtMs,
            LastHeartbeatAtMs,
            EndpointIds,
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
                            "agentId" | "agent_id" => Ok(GeneratedField::AgentId),
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "generation" => Ok(GeneratedField::Generation),
                            "registeredAtMs" | "registered_at_ms" => Ok(GeneratedField::RegisteredAtMs),
                            "lastHeartbeatAtMs" | "last_heartbeat_at_ms" => Ok(GeneratedField::LastHeartbeatAtMs),
                            "endpointIds" | "endpoint_ids" => Ok(GeneratedField::EndpointIds),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AgentLinkSummary;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.AgentLinkSummary")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AgentLinkSummary, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut agent_id__ = None;
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
                let mut generation__ = None;
                let mut registered_at_ms__ = None;
                let mut last_heartbeat_at_ms__ = None;
                let mut endpoint_ids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::AgentId => {
                            if agent_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("agentId"));
                            }
                            agent_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Generation => {
                            if generation__.is_some() {
                                return Err(serde::de::Error::duplicate_field("generation"));
                            }
                            generation__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RegisteredAtMs => {
                            if registered_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("registeredAtMs"));
                            }
                            registered_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastHeartbeatAtMs => {
                            if last_heartbeat_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastHeartbeatAtMs"));
                            }
                            last_heartbeat_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::EndpointIds => {
                            if endpoint_ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointIds"));
                            }
                            endpoint_ids__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(AgentLinkSummary {
                    agent_id: agent_id__.unwrap_or_default(),
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    generation: generation__.unwrap_or_default(),
                    registered_at_ms: registered_at_ms__.unwrap_or_default(),
                    last_heartbeat_at_ms: last_heartbeat_at_ms__.unwrap_or_default(),
                    endpoint_ids: endpoint_ids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.AgentLinkSummary", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for AgentRegisterRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.agent_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if !self.endpoints.is_empty() {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if self.host_origin.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.AgentRegisterRequest", len)?;
        if !self.agent_id.is_empty() {
            struct_ser.serialize_field("agentId", &self.agent_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if !self.endpoints.is_empty() {
            struct_ser.serialize_field("endpoints", &self.endpoints)?;
        }
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if let Some(v) = self.host_origin.as_ref() {
            struct_ser.serialize_field("hostOrigin", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AgentRegisterRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "agent_id",
            "agentId",
            "tenant_id",
            "tenantId",
            "endpoints",
            "provides",
            "host_origin",
            "hostOrigin",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            AgentId,
            TenantId,
            Endpoints,
            Provides,
            HostOrigin,
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
                            "agentId" | "agent_id" => Ok(GeneratedField::AgentId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "endpoints" => Ok(GeneratedField::Endpoints),
                            "provides" => Ok(GeneratedField::Provides),
                            "hostOrigin" | "host_origin" => Ok(GeneratedField::HostOrigin),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AgentRegisterRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.AgentRegisterRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AgentRegisterRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut agent_id__ = None;
                let mut tenant_id__ = None;
                let mut endpoints__ = None;
                let mut provides__ = None;
                let mut host_origin__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::AgentId => {
                            if agent_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("agentId"));
                            }
                            agent_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Endpoints => {
                            if endpoints__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpoints"));
                            }
                            endpoints__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Provides => {
                            if provides__.is_some() {
                                return Err(serde::de::Error::duplicate_field("provides"));
                            }
                            provides__ = Some(map_.next_value()?);
                        }
                        GeneratedField::HostOrigin => {
                            if host_origin__.is_some() {
                                return Err(serde::de::Error::duplicate_field("hostOrigin"));
                            }
                            host_origin__ = map_.next_value()?;
                        }
                    }
                }
                Ok(AgentRegisterRequest {
                    agent_id: agent_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    endpoints: endpoints__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    host_origin: host_origin__,
                })
            }
        }
        deserializer.deserialize_struct("conex.AgentRegisterRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for AgentRegisterResult {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.agent_id.is_empty() {
            len += 1;
        }
        if !self.link_id.is_empty() {
            len += 1;
        }
        if !self.accepted_endpoint_ids.is_empty() {
            len += 1;
        }
        if !self.rejected_capabilities.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.AgentRegisterResult", len)?;
        if !self.agent_id.is_empty() {
            struct_ser.serialize_field("agentId", &self.agent_id)?;
        }
        if !self.link_id.is_empty() {
            struct_ser.serialize_field("linkId", &self.link_id)?;
        }
        if !self.accepted_endpoint_ids.is_empty() {
            struct_ser.serialize_field("acceptedEndpointIds", &self.accepted_endpoint_ids)?;
        }
        if !self.rejected_capabilities.is_empty() {
            struct_ser.serialize_field("rejectedCapabilities", &self.rejected_capabilities)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AgentRegisterResult {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "agent_id",
            "agentId",
            "link_id",
            "linkId",
            "accepted_endpoint_ids",
            "acceptedEndpointIds",
            "rejected_capabilities",
            "rejectedCapabilities",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            AgentId,
            LinkId,
            AcceptedEndpointIds,
            RejectedCapabilities,
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
                            "agentId" | "agent_id" => Ok(GeneratedField::AgentId),
                            "linkId" | "link_id" => Ok(GeneratedField::LinkId),
                            "acceptedEndpointIds" | "accepted_endpoint_ids" => Ok(GeneratedField::AcceptedEndpointIds),
                            "rejectedCapabilities" | "rejected_capabilities" => Ok(GeneratedField::RejectedCapabilities),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AgentRegisterResult;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.AgentRegisterResult")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AgentRegisterResult, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut agent_id__ = None;
                let mut link_id__ = None;
                let mut accepted_endpoint_ids__ = None;
                let mut rejected_capabilities__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::AgentId => {
                            if agent_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("agentId"));
                            }
                            agent_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LinkId => {
                            if link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("linkId"));
                            }
                            link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AcceptedEndpointIds => {
                            if accepted_endpoint_ids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("acceptedEndpointIds"));
                            }
                            accepted_endpoint_ids__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RejectedCapabilities => {
                            if rejected_capabilities__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rejectedCapabilities"));
                            }
                            rejected_capabilities__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(AgentRegisterResult {
                    agent_id: agent_id__.unwrap_or_default(),
                    link_id: link_id__.unwrap_or_default(),
                    accepted_endpoint_ids: accepted_endpoint_ids__.unwrap_or_default(),
                    rejected_capabilities: rejected_capabilities__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.AgentRegisterResult", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for AttachmentKind {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "ATTACHMENT_KIND_UNSPECIFIED",
            Self::Embedded => "ATTACHMENT_KIND_EMBEDDED",
            Self::OutboundHttp => "ATTACHMENT_KIND_OUTBOUND_HTTP",
            Self::ReverseAgent => "ATTACHMENT_KIND_REVERSE_AGENT",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for AttachmentKind {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ATTACHMENT_KIND_UNSPECIFIED",
            "ATTACHMENT_KIND_EMBEDDED",
            "ATTACHMENT_KIND_OUTBOUND_HTTP",
            "ATTACHMENT_KIND_REVERSE_AGENT",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = AttachmentKind;

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
                    "ATTACHMENT_KIND_UNSPECIFIED" => Ok(AttachmentKind::Unspecified),
                    "ATTACHMENT_KIND_EMBEDDED" => Ok(AttachmentKind::Embedded),
                    "ATTACHMENT_KIND_OUTBOUND_HTTP" => Ok(AttachmentKind::OutboundHttp),
                    "ATTACHMENT_KIND_REVERSE_AGENT" => Ok(AttachmentKind::ReverseAgent),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for AuthorizedScope {
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
        if !self.root.is_empty() {
            len += 1;
        }
        if self.subtree {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.AuthorizedScope", len)?;
        if !self.method.is_empty() {
            struct_ser.serialize_field("method", &self.method)?;
        }
        if !self.root.is_empty() {
            struct_ser.serialize_field("root", &self.root)?;
        }
        if self.subtree {
            struct_ser.serialize_field("subtree", &self.subtree)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AuthorizedScope {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "method",
            "root",
            "subtree",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Method,
            Root,
            Subtree,
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
                            "root" => Ok(GeneratedField::Root),
                            "subtree" => Ok(GeneratedField::Subtree),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AuthorizedScope;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.AuthorizedScope")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AuthorizedScope, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut method__ = None;
                let mut root__ = None;
                let mut subtree__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Method => {
                            if method__.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Subtree => {
                            if subtree__.is_some() {
                                return Err(serde::de::Error::duplicate_field("subtree"));
                            }
                            subtree__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(AuthorizedScope {
                    method: method__.unwrap_or_default(),
                    root: root__.unwrap_or_default(),
                    subtree: subtree__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.AuthorizedScope", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobAccess {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.endpoint_id.is_empty() {
            len += 1;
        }
        if !self.plane.is_empty() {
            len += 1;
        }
        if self.space_id.is_some() {
            len += 1;
        }
        if !self.resource_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobAccess", len)?;
        if !self.endpoint_id.is_empty() {
            struct_ser.serialize_field("endpointId", &self.endpoint_id)?;
        }
        if !self.plane.is_empty() {
            struct_ser.serialize_field("plane", &self.plane)?;
        }
        if let Some(v) = self.space_id.as_ref() {
            struct_ser.serialize_field("spaceId", v)?;
        }
        if !self.resource_id.is_empty() {
            struct_ser.serialize_field("resourceId", &self.resource_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobAccess {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "endpoint_id",
            "endpointId",
            "plane",
            "space_id",
            "spaceId",
            "resource_id",
            "resourceId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EndpointId,
            Plane,
            SpaceId,
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
                            "endpointId" | "endpoint_id" => Ok(GeneratedField::EndpointId),
                            "plane" => Ok(GeneratedField::Plane),
                            "spaceId" | "space_id" => Ok(GeneratedField::SpaceId),
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
            type Value = BlobAccess;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobAccess")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobAccess, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut endpoint_id__ = None;
                let mut plane__ = None;
                let mut space_id__ = None;
                let mut resource_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EndpointId => {
                            if endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointId"));
                            }
                            endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Plane => {
                            if plane__.is_some() {
                                return Err(serde::de::Error::duplicate_field("plane"));
                            }
                            plane__ = Some(map_.next_value()?);
                        }
                        GeneratedField::SpaceId => {
                            if space_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("spaceId"));
                            }
                            space_id__ = map_.next_value()?;
                        }
                        GeneratedField::ResourceId => {
                            if resource_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceId"));
                            }
                            resource_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobAccess {
                    endpoint_id: endpoint_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    space_id: space_id__,
                    resource_id: resource_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobAccess", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobCancelRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.upload_id.is_empty() {
            len += 1;
        }
        if self.reason.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobCancelRequest", len)?;
        if !self.upload_id.is_empty() {
            struct_ser.serialize_field("uploadId", &self.upload_id)?;
        }
        if let Some(v) = self.reason.as_ref() {
            struct_ser.serialize_field("reason", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobCancelRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "upload_id",
            "uploadId",
            "reason",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            UploadId,
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
                            "uploadId" | "upload_id" => Ok(GeneratedField::UploadId),
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
            type Value = BlobCancelRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobCancelRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobCancelRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut upload_id__ = None;
                let mut reason__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::UploadId => {
                            if upload_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("uploadId"));
                            }
                            upload_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = map_.next_value()?;
                        }
                    }
                }
                Ok(BlobCancelRequest {
                    upload_id: upload_id__.unwrap_or_default(),
                    reason: reason__,
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobCancelRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobCancelResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("conex.BlobCancelResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobCancelResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobCancelResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobCancelResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobCancelResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(BlobCancelResponse {
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobCancelResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobChunkRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.upload_id.is_empty() {
            len += 1;
        }
        if self.chunk_index != 0 {
            len += 1;
        }
        if !self.chunk_cid.is_empty() {
            len += 1;
        }
        if !self.chunk_bytes.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobChunkRequest", len)?;
        if !self.upload_id.is_empty() {
            struct_ser.serialize_field("uploadId", &self.upload_id)?;
        }
        if self.chunk_index != 0 {
            struct_ser.serialize_field("chunkIndex", &self.chunk_index)?;
        }
        if !self.chunk_cid.is_empty() {
            struct_ser.serialize_field("chunkCid", &self.chunk_cid)?;
        }
        if !self.chunk_bytes.is_empty() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("chunkBytes", pbjson::private::base64::encode(&self.chunk_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobChunkRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "upload_id",
            "uploadId",
            "chunk_index",
            "chunkIndex",
            "chunk_cid",
            "chunkCid",
            "chunk_bytes",
            "chunkBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            UploadId,
            ChunkIndex,
            ChunkCid,
            ChunkBytes,
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
                            "uploadId" | "upload_id" => Ok(GeneratedField::UploadId),
                            "chunkIndex" | "chunk_index" => Ok(GeneratedField::ChunkIndex),
                            "chunkCid" | "chunk_cid" => Ok(GeneratedField::ChunkCid),
                            "chunkBytes" | "chunk_bytes" => Ok(GeneratedField::ChunkBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobChunkRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobChunkRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobChunkRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut upload_id__ = None;
                let mut chunk_index__ = None;
                let mut chunk_cid__ = None;
                let mut chunk_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::UploadId => {
                            if upload_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("uploadId"));
                            }
                            upload_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ChunkIndex => {
                            if chunk_index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkIndex"));
                            }
                            chunk_index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ChunkCid => {
                            if chunk_cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkCid"));
                            }
                            chunk_cid__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ChunkBytes => {
                            if chunk_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkBytes"));
                            }
                            chunk_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(BlobChunkRequest {
                    upload_id: upload_id__.unwrap_or_default(),
                    chunk_index: chunk_index__.unwrap_or_default(),
                    chunk_cid: chunk_cid__.unwrap_or_default(),
                    chunk_bytes: chunk_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobChunkRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobChunkResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.chunk_index != 0 {
            len += 1;
        }
        if self.received_bytes != 0 {
            len += 1;
        }
        if self.remaining_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobChunkResponse", len)?;
        if self.chunk_index != 0 {
            struct_ser.serialize_field("chunkIndex", &self.chunk_index)?;
        }
        if self.received_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("receivedBytes", ToString::to_string(&self.received_bytes).as_str())?;
        }
        if self.remaining_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("remainingBytes", ToString::to_string(&self.remaining_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobChunkResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "chunk_index",
            "chunkIndex",
            "received_bytes",
            "receivedBytes",
            "remaining_bytes",
            "remainingBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ChunkIndex,
            ReceivedBytes,
            RemainingBytes,
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
                            "chunkIndex" | "chunk_index" => Ok(GeneratedField::ChunkIndex),
                            "receivedBytes" | "received_bytes" => Ok(GeneratedField::ReceivedBytes),
                            "remainingBytes" | "remaining_bytes" => Ok(GeneratedField::RemainingBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobChunkResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobChunkResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobChunkResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut chunk_index__ = None;
                let mut received_bytes__ = None;
                let mut remaining_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ChunkIndex => {
                            if chunk_index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkIndex"));
                            }
                            chunk_index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ReceivedBytes => {
                            if received_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receivedBytes"));
                            }
                            received_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RemainingBytes => {
                            if remaining_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remainingBytes"));
                            }
                            remaining_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(BlobChunkResponse {
                    chunk_index: chunk_index__.unwrap_or_default(),
                    received_bytes: received_bytes__.unwrap_or_default(),
                    remaining_bytes: remaining_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobChunkResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobCommitRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.upload_id.is_empty() {
            len += 1;
        }
        if self.declared_root.is_some() {
            len += 1;
        }
        if self.pin_until_ms.is_some() {
            len += 1;
        }
        if !self.persistence.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobCommitRequest", len)?;
        if !self.upload_id.is_empty() {
            struct_ser.serialize_field("uploadId", &self.upload_id)?;
        }
        if let Some(v) = self.declared_root.as_ref() {
            struct_ser.serialize_field("declaredRoot", v)?;
        }
        if let Some(v) = self.pin_until_ms.as_ref() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("pinUntilMs", ToString::to_string(&v).as_str())?;
        }
        if !self.persistence.is_empty() {
            struct_ser.serialize_field("persistence", &self.persistence)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobCommitRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "upload_id",
            "uploadId",
            "declared_root",
            "declaredRoot",
            "pin_until_ms",
            "pinUntilMs",
            "persistence",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            UploadId,
            DeclaredRoot,
            PinUntilMs,
            Persistence,
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
                            "uploadId" | "upload_id" => Ok(GeneratedField::UploadId),
                            "declaredRoot" | "declared_root" => Ok(GeneratedField::DeclaredRoot),
                            "pinUntilMs" | "pin_until_ms" => Ok(GeneratedField::PinUntilMs),
                            "persistence" => Ok(GeneratedField::Persistence),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobCommitRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobCommitRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobCommitRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut upload_id__ = None;
                let mut declared_root__ = None;
                let mut pin_until_ms__ = None;
                let mut persistence__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::UploadId => {
                            if upload_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("uploadId"));
                            }
                            upload_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DeclaredRoot => {
                            if declared_root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("declaredRoot"));
                            }
                            declared_root__ = map_.next_value()?;
                        }
                        GeneratedField::PinUntilMs => {
                            if pin_until_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pinUntilMs"));
                            }
                            pin_until_ms__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::Persistence => {
                            if persistence__.is_some() {
                                return Err(serde::de::Error::duplicate_field("persistence"));
                            }
                            persistence__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobCommitRequest {
                    upload_id: upload_id__.unwrap_or_default(),
                    declared_root: declared_root__,
                    pin_until_ms: pin_until_ms__,
                    persistence: persistence__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobCommitRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobCommitResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.resource_root_cid.is_empty() {
            len += 1;
        }
        if self.committed_bytes != 0 {
            len += 1;
        }
        if !self.persistence.is_empty() {
            len += 1;
        }
        if !self.receipt_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobCommitResponse", len)?;
        if !self.resource_root_cid.is_empty() {
            struct_ser.serialize_field("resourceRootCid", &self.resource_root_cid)?;
        }
        if self.committed_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("committedBytes", ToString::to_string(&self.committed_bytes).as_str())?;
        }
        if !self.persistence.is_empty() {
            struct_ser.serialize_field("persistence", &self.persistence)?;
        }
        if !self.receipt_id.is_empty() {
            struct_ser.serialize_field("receiptId", &self.receipt_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobCommitResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "resource_root_cid",
            "resourceRootCid",
            "committed_bytes",
            "committedBytes",
            "persistence",
            "receipt_id",
            "receiptId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ResourceRootCid,
            CommittedBytes,
            Persistence,
            ReceiptId,
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
                            "resourceRootCid" | "resource_root_cid" => Ok(GeneratedField::ResourceRootCid),
                            "committedBytes" | "committed_bytes" => Ok(GeneratedField::CommittedBytes),
                            "persistence" => Ok(GeneratedField::Persistence),
                            "receiptId" | "receipt_id" => Ok(GeneratedField::ReceiptId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobCommitResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobCommitResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobCommitResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource_root_cid__ = None;
                let mut committed_bytes__ = None;
                let mut persistence__ = None;
                let mut receipt_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ResourceRootCid => {
                            if resource_root_cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceRootCid"));
                            }
                            resource_root_cid__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CommittedBytes => {
                            if committed_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("committedBytes"));
                            }
                            committed_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Persistence => {
                            if persistence__.is_some() {
                                return Err(serde::de::Error::duplicate_field("persistence"));
                            }
                            persistence__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ReceiptId => {
                            if receipt_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiptId"));
                            }
                            receipt_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobCommitResponse {
                    resource_root_cid: resource_root_cid__.unwrap_or_default(),
                    committed_bytes: committed_bytes__.unwrap_or_default(),
                    persistence: persistence__.unwrap_or_default(),
                    receipt_id: receipt_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobCommitResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobGetRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.target.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobGetRequest", len)?;
        if let Some(v) = self.target.as_ref() {
            match v {
                blob_get_request::Target::Committed(v) => {
                    struct_ser.serialize_field("committed", v)?;
                }
                blob_get_request::Target::Remote(v) => {
                    struct_ser.serialize_field("remote", v)?;
                }
            }
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobGetRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "committed",
            "remote",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Committed,
            Remote,
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
                            "committed" => Ok(GeneratedField::Committed),
                            "remote" => Ok(GeneratedField::Remote),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobGetRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobGetRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobGetRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut target__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Committed => {
                            if target__.is_some() {
                                return Err(serde::de::Error::duplicate_field("committed"));
                            }
                            target__ = map_.next_value::<::std::option::Option<_>>()?.map(blob_get_request::Target::Committed)
;
                        }
                        GeneratedField::Remote => {
                            if target__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remote"));
                            }
                            target__ = map_.next_value::<::std::option::Option<_>>()?.map(blob_get_request::Target::Remote)
;
                        }
                    }
                }
                Ok(BlobGetRequest {
                    target: target__,
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobGetRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobGetResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.chunk_bytes.is_empty() {
            len += 1;
        }
        if self.chunk_cid.is_some() {
            len += 1;
        }
        if self.revision.is_some() {
            len += 1;
        }
        if self.eof {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobGetResponse", len)?;
        if !self.chunk_bytes.is_empty() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("chunkBytes", pbjson::private::base64::encode(&self.chunk_bytes).as_str())?;
        }
        if let Some(v) = self.chunk_cid.as_ref() {
            struct_ser.serialize_field("chunkCid", v)?;
        }
        if let Some(v) = self.revision.as_ref() {
            struct_ser.serialize_field("revision", v)?;
        }
        if self.eof {
            struct_ser.serialize_field("eof", &self.eof)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobGetResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "chunk_bytes",
            "chunkBytes",
            "chunk_cid",
            "chunkCid",
            "revision",
            "eof",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ChunkBytes,
            ChunkCid,
            Revision,
            Eof,
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
                            "chunkBytes" | "chunk_bytes" => Ok(GeneratedField::ChunkBytes),
                            "chunkCid" | "chunk_cid" => Ok(GeneratedField::ChunkCid),
                            "revision" => Ok(GeneratedField::Revision),
                            "eof" => Ok(GeneratedField::Eof),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobGetResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobGetResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobGetResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut chunk_bytes__ = None;
                let mut chunk_cid__ = None;
                let mut revision__ = None;
                let mut eof__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ChunkBytes => {
                            if chunk_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkBytes"));
                            }
                            chunk_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ChunkCid => {
                            if chunk_cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkCid"));
                            }
                            chunk_cid__ = map_.next_value()?;
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = map_.next_value()?;
                        }
                        GeneratedField::Eof => {
                            if eof__.is_some() {
                                return Err(serde::de::Error::duplicate_field("eof"));
                            }
                            eof__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobGetResponse {
                    chunk_bytes: chunk_bytes__.unwrap_or_default(),
                    chunk_cid: chunk_cid__,
                    revision: revision__,
                    eof: eof__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobGetResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobHaveRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.chunk_cids.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobHaveRequest", len)?;
        if !self.chunk_cids.is_empty() {
            struct_ser.serialize_field("chunkCids", &self.chunk_cids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobHaveRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "chunk_cids",
            "chunkCids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ChunkCids,
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
                            "chunkCids" | "chunk_cids" => Ok(GeneratedField::ChunkCids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobHaveRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobHaveRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobHaveRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut chunk_cids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ChunkCids => {
                            if chunk_cids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkCids"));
                            }
                            chunk_cids__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobHaveRequest {
                    chunk_cids: chunk_cids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobHaveRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobHaveResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.present.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobHaveResponse", len)?;
        if !self.present.is_empty() {
            struct_ser.serialize_field("present", &self.present)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobHaveResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "present",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Present,
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
                            "present" => Ok(GeneratedField::Present),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobHaveResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobHaveResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobHaveResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut present__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Present => {
                            if present__.is_some() {
                                return Err(serde::de::Error::duplicate_field("present"));
                            }
                            present__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobHaveResponse {
                    present: present__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobHaveResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobPinRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.root.is_some() {
            len += 1;
        }
        if self.pin_until_ms.is_some() {
            len += 1;
        }
        if self.space_id.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobPinRequest", len)?;
        if let Some(v) = self.root.as_ref() {
            struct_ser.serialize_field("root", v)?;
        }
        if let Some(v) = self.pin_until_ms.as_ref() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("pinUntilMs", ToString::to_string(&v).as_str())?;
        }
        if let Some(v) = self.space_id.as_ref() {
            struct_ser.serialize_field("spaceId", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobPinRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "root",
            "pin_until_ms",
            "pinUntilMs",
            "space_id",
            "spaceId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Root,
            PinUntilMs,
            SpaceId,
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
                            "pinUntilMs" | "pin_until_ms" => Ok(GeneratedField::PinUntilMs),
                            "spaceId" | "space_id" => Ok(GeneratedField::SpaceId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobPinRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobPinRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobPinRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut root__ = None;
                let mut pin_until_ms__ = None;
                let mut space_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = map_.next_value()?;
                        }
                        GeneratedField::PinUntilMs => {
                            if pin_until_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pinUntilMs"));
                            }
                            pin_until_ms__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::SpaceId => {
                            if space_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("spaceId"));
                            }
                            space_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(BlobPinRequest {
                    root: root__,
                    pin_until_ms: pin_until_ms__,
                    space_id: space_id__,
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobPinRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobPinResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.pin_id.is_empty() {
            len += 1;
        }
        if self.expires_at_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobPinResponse", len)?;
        if !self.pin_id.is_empty() {
            struct_ser.serialize_field("pinId", &self.pin_id)?;
        }
        if self.expires_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expiresAtMs", ToString::to_string(&self.expires_at_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobPinResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "pin_id",
            "pinId",
            "expires_at_ms",
            "expiresAtMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            PinId,
            ExpiresAtMs,
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
                            "pinId" | "pin_id" => Ok(GeneratedField::PinId),
                            "expiresAtMs" | "expires_at_ms" => Ok(GeneratedField::ExpiresAtMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobPinResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobPinResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobPinResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut pin_id__ = None;
                let mut expires_at_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::PinId => {
                            if pin_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pinId"));
                            }
                            pin_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpiresAtMs => {
                            if expires_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expiresAtMs"));
                            }
                            expires_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(BlobPinResponse {
                    pin_id: pin_id__.unwrap_or_default(),
                    expires_at_ms: expires_at_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobPinResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobPutRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.format_version != 0 {
            len += 1;
        }
        if !self.declared_size_bytes.is_empty() {
            len += 1;
        }
        if !self.declared_chunk_size.is_empty() {
            len += 1;
        }
        if self.expected_root.is_some() {
            len += 1;
        }
        if self.access.is_some() {
            len += 1;
        }
        if self.requested_lease_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobPutRequest", len)?;
        if self.format_version != 0 {
            struct_ser.serialize_field("formatVersion", &self.format_version)?;
        }
        if !self.declared_size_bytes.is_empty() {
            struct_ser.serialize_field("declaredSizeBytes", &self.declared_size_bytes)?;
        }
        if !self.declared_chunk_size.is_empty() {
            struct_ser.serialize_field("declaredChunkSize", &self.declared_chunk_size)?;
        }
        if let Some(v) = self.expected_root.as_ref() {
            struct_ser.serialize_field("expectedRoot", v)?;
        }
        if let Some(v) = self.access.as_ref() {
            struct_ser.serialize_field("access", v)?;
        }
        if self.requested_lease_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("requestedLeaseMs", ToString::to_string(&self.requested_lease_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobPutRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "format_version",
            "formatVersion",
            "declared_size_bytes",
            "declaredSizeBytes",
            "declared_chunk_size",
            "declaredChunkSize",
            "expected_root",
            "expectedRoot",
            "access",
            "requested_lease_ms",
            "requestedLeaseMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            FormatVersion,
            DeclaredSizeBytes,
            DeclaredChunkSize,
            ExpectedRoot,
            Access,
            RequestedLeaseMs,
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
                            "formatVersion" | "format_version" => Ok(GeneratedField::FormatVersion),
                            "declaredSizeBytes" | "declared_size_bytes" => Ok(GeneratedField::DeclaredSizeBytes),
                            "declaredChunkSize" | "declared_chunk_size" => Ok(GeneratedField::DeclaredChunkSize),
                            "expectedRoot" | "expected_root" => Ok(GeneratedField::ExpectedRoot),
                            "access" => Ok(GeneratedField::Access),
                            "requestedLeaseMs" | "requested_lease_ms" => Ok(GeneratedField::RequestedLeaseMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobPutRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobPutRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobPutRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut format_version__ = None;
                let mut declared_size_bytes__ = None;
                let mut declared_chunk_size__ = None;
                let mut expected_root__ = None;
                let mut access__ = None;
                let mut requested_lease_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::FormatVersion => {
                            if format_version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("formatVersion"));
                            }
                            format_version__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::DeclaredSizeBytes => {
                            if declared_size_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("declaredSizeBytes"));
                            }
                            declared_size_bytes__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DeclaredChunkSize => {
                            if declared_chunk_size__.is_some() {
                                return Err(serde::de::Error::duplicate_field("declaredChunkSize"));
                            }
                            declared_chunk_size__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpectedRoot => {
                            if expected_root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expectedRoot"));
                            }
                            expected_root__ = map_.next_value()?;
                        }
                        GeneratedField::Access => {
                            if access__.is_some() {
                                return Err(serde::de::Error::duplicate_field("access"));
                            }
                            access__ = map_.next_value()?;
                        }
                        GeneratedField::RequestedLeaseMs => {
                            if requested_lease_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestedLeaseMs"));
                            }
                            requested_lease_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(BlobPutRequest {
                    format_version: format_version__.unwrap_or_default(),
                    declared_size_bytes: declared_size_bytes__.unwrap_or_default(),
                    declared_chunk_size: declared_chunk_size__.unwrap_or_default(),
                    expected_root: expected_root__,
                    access: access__,
                    requested_lease_ms: requested_lease_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobPutRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobPutResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.upload_id.is_empty() {
            len += 1;
        }
        if self.chunk_size != 0 {
            len += 1;
        }
        if self.lease_ms != 0 {
            len += 1;
        }
        if self.max_blob_bytes != 0 {
            len += 1;
        }
        if self.inline_threshold_bytes != 0 {
            len += 1;
        }
        if !self.already_have_chunk_cids.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobPutResponse", len)?;
        if !self.upload_id.is_empty() {
            struct_ser.serialize_field("uploadId", &self.upload_id)?;
        }
        if self.chunk_size != 0 {
            struct_ser.serialize_field("chunkSize", &self.chunk_size)?;
        }
        if self.lease_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("leaseMs", ToString::to_string(&self.lease_ms).as_str())?;
        }
        if self.max_blob_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("maxBlobBytes", ToString::to_string(&self.max_blob_bytes).as_str())?;
        }
        if self.inline_threshold_bytes != 0 {
            struct_ser.serialize_field("inlineThresholdBytes", &self.inline_threshold_bytes)?;
        }
        if !self.already_have_chunk_cids.is_empty() {
            struct_ser.serialize_field("alreadyHaveChunkCids", &self.already_have_chunk_cids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobPutResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "upload_id",
            "uploadId",
            "chunk_size",
            "chunkSize",
            "lease_ms",
            "leaseMs",
            "max_blob_bytes",
            "maxBlobBytes",
            "inline_threshold_bytes",
            "inlineThresholdBytes",
            "already_have_chunk_cids",
            "alreadyHaveChunkCids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            UploadId,
            ChunkSize,
            LeaseMs,
            MaxBlobBytes,
            InlineThresholdBytes,
            AlreadyHaveChunkCids,
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
                            "uploadId" | "upload_id" => Ok(GeneratedField::UploadId),
                            "chunkSize" | "chunk_size" => Ok(GeneratedField::ChunkSize),
                            "leaseMs" | "lease_ms" => Ok(GeneratedField::LeaseMs),
                            "maxBlobBytes" | "max_blob_bytes" => Ok(GeneratedField::MaxBlobBytes),
                            "inlineThresholdBytes" | "inline_threshold_bytes" => Ok(GeneratedField::InlineThresholdBytes),
                            "alreadyHaveChunkCids" | "already_have_chunk_cids" => Ok(GeneratedField::AlreadyHaveChunkCids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobPutResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobPutResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobPutResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut upload_id__ = None;
                let mut chunk_size__ = None;
                let mut lease_ms__ = None;
                let mut max_blob_bytes__ = None;
                let mut inline_threshold_bytes__ = None;
                let mut already_have_chunk_cids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::UploadId => {
                            if upload_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("uploadId"));
                            }
                            upload_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ChunkSize => {
                            if chunk_size__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkSize"));
                            }
                            chunk_size__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LeaseMs => {
                            if lease_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leaseMs"));
                            }
                            lease_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MaxBlobBytes => {
                            if max_blob_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxBlobBytes"));
                            }
                            max_blob_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::InlineThresholdBytes => {
                            if inline_threshold_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("inlineThresholdBytes"));
                            }
                            inline_threshold_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::AlreadyHaveChunkCids => {
                            if already_have_chunk_cids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("alreadyHaveChunkCids"));
                            }
                            already_have_chunk_cids__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobPutResponse {
                    upload_id: upload_id__.unwrap_or_default(),
                    chunk_size: chunk_size__.unwrap_or_default(),
                    lease_ms: lease_ms__.unwrap_or_default(),
                    max_blob_bytes: max_blob_bytes__.unwrap_or_default(),
                    inline_threshold_bytes: inline_threshold_bytes__.unwrap_or_default(),
                    already_have_chunk_cids: already_have_chunk_cids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobPutResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobRef {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.cid.is_some() {
            len += 1;
        }
        if !self.size_bytes.is_empty() {
            len += 1;
        }
        if !self.mime.is_empty() {
            len += 1;
        }
        if self.access.is_some() {
            len += 1;
        }
        if self.revision.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobRef", len)?;
        if let Some(v) = self.cid.as_ref() {
            struct_ser.serialize_field("cid", v)?;
        }
        if !self.size_bytes.is_empty() {
            struct_ser.serialize_field("sizeBytes", &self.size_bytes)?;
        }
        if !self.mime.is_empty() {
            struct_ser.serialize_field("mime", &self.mime)?;
        }
        if let Some(v) = self.access.as_ref() {
            struct_ser.serialize_field("access", v)?;
        }
        if let Some(v) = self.revision.as_ref() {
            struct_ser.serialize_field("revision", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobRef {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "cid",
            "size_bytes",
            "sizeBytes",
            "mime",
            "access",
            "revision",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Cid,
            SizeBytes,
            Mime,
            Access,
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
                            "cid" => Ok(GeneratedField::Cid),
                            "sizeBytes" | "size_bytes" => Ok(GeneratedField::SizeBytes),
                            "mime" => Ok(GeneratedField::Mime),
                            "access" => Ok(GeneratedField::Access),
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
            type Value = BlobRef;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobRef")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobRef, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut cid__ = None;
                let mut size_bytes__ = None;
                let mut mime__ = None;
                let mut access__ = None;
                let mut revision__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Cid => {
                            if cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("cid"));
                            }
                            cid__ = map_.next_value()?;
                        }
                        GeneratedField::SizeBytes => {
                            if size_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sizeBytes"));
                            }
                            size_bytes__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Mime => {
                            if mime__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mime"));
                            }
                            mime__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Access => {
                            if access__.is_some() {
                                return Err(serde::de::Error::duplicate_field("access"));
                            }
                            access__ = map_.next_value()?;
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = map_.next_value()?;
                        }
                    }
                }
                Ok(BlobRef {
                    cid: cid__,
                    size_bytes: size_bytes__.unwrap_or_default(),
                    mime: mime__.unwrap_or_default(),
                    access: access__,
                    revision: revision__,
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobRef", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobUnpinRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.pin_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.BlobUnpinRequest", len)?;
        if !self.pin_id.is_empty() {
            struct_ser.serialize_field("pinId", &self.pin_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobUnpinRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "pin_id",
            "pinId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            PinId,
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
                            "pinId" | "pin_id" => Ok(GeneratedField::PinId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobUnpinRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobUnpinRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobUnpinRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut pin_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::PinId => {
                            if pin_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pinId"));
                            }
                            pin_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(BlobUnpinRequest {
                    pin_id: pin_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobUnpinRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for BlobUnpinResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("conex.BlobUnpinResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for BlobUnpinResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = BlobUnpinResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.BlobUnpinResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<BlobUnpinResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(BlobUnpinResponse {
                })
            }
        }
        deserializer.deserialize_struct("conex.BlobUnpinResponse", FIELDS, GeneratedVisitor)
    }
}
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
        let mut struct_ser = serializer.serialize_struct("conex.CallParams", len)?;
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
                formatter.write_str("struct conex.CallParams")
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
        deserializer.deserialize_struct("conex.CallParams", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ChunkEntry {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.chunk_cid.is_empty() {
            len += 1;
        }
        if !self.chunk_length.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ChunkEntry", len)?;
        if !self.chunk_cid.is_empty() {
            struct_ser.serialize_field("chunkCid", &self.chunk_cid)?;
        }
        if !self.chunk_length.is_empty() {
            struct_ser.serialize_field("chunkLength", &self.chunk_length)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ChunkEntry {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "chunk_cid",
            "chunkCid",
            "chunk_length",
            "chunkLength",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ChunkCid,
            ChunkLength,
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
                            "chunkCid" | "chunk_cid" => Ok(GeneratedField::ChunkCid),
                            "chunkLength" | "chunk_length" => Ok(GeneratedField::ChunkLength),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ChunkEntry;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ChunkEntry")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ChunkEntry, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut chunk_cid__ = None;
                let mut chunk_length__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ChunkCid => {
                            if chunk_cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkCid"));
                            }
                            chunk_cid__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ChunkLength => {
                            if chunk_length__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkLength"));
                            }
                            chunk_length__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ChunkEntry {
                    chunk_cid: chunk_cid__.unwrap_or_default(),
                    chunk_length: chunk_length__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ChunkEntry", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ChunkManifest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.format_version != 0 {
            len += 1;
        }
        if self.chunk_size != 0 {
            len += 1;
        }
        if !self.content_length.is_empty() {
            len += 1;
        }
        if self.root.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ChunkManifest", len)?;
        if self.format_version != 0 {
            struct_ser.serialize_field("formatVersion", &self.format_version)?;
        }
        if self.chunk_size != 0 {
            struct_ser.serialize_field("chunkSize", &self.chunk_size)?;
        }
        if !self.content_length.is_empty() {
            struct_ser.serialize_field("contentLength", &self.content_length)?;
        }
        if let Some(v) = self.root.as_ref() {
            struct_ser.serialize_field("root", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ChunkManifest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "format_version",
            "formatVersion",
            "chunk_size",
            "chunkSize",
            "content_length",
            "contentLength",
            "root",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            FormatVersion,
            ChunkSize,
            ContentLength,
            Root,
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
                            "formatVersion" | "format_version" => Ok(GeneratedField::FormatVersion),
                            "chunkSize" | "chunk_size" => Ok(GeneratedField::ChunkSize),
                            "contentLength" | "content_length" => Ok(GeneratedField::ContentLength),
                            "root" => Ok(GeneratedField::Root),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ChunkManifest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ChunkManifest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ChunkManifest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut format_version__ = None;
                let mut chunk_size__ = None;
                let mut content_length__ = None;
                let mut root__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::FormatVersion => {
                            if format_version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("formatVersion"));
                            }
                            format_version__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ChunkSize => {
                            if chunk_size__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkSize"));
                            }
                            chunk_size__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ContentLength => {
                            if content_length__.is_some() {
                                return Err(serde::de::Error::duplicate_field("contentLength"));
                            }
                            content_length__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ChunkManifest {
                    format_version: format_version__.unwrap_or_default(),
                    chunk_size: chunk_size__.unwrap_or_default(),
                    content_length: content_length__.unwrap_or_default(),
                    root: root__,
                })
            }
        }
        deserializer.deserialize_struct("conex.ChunkManifest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientHelloRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.target_link_id.is_empty() {
            len += 1;
        }
        if !self.text.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientHelloRequest", len)?;
        if !self.target_link_id.is_empty() {
            struct_ser.serialize_field("targetLinkId", &self.target_link_id)?;
        }
        if !self.text.is_empty() {
            struct_ser.serialize_field("text", &self.text)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientHelloRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "target_link_id",
            "targetLinkId",
            "text",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TargetLinkId,
            Text,
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
                            "targetLinkId" | "target_link_id" => Ok(GeneratedField::TargetLinkId),
                            "text" => Ok(GeneratedField::Text),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientHelloRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientHelloRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientHelloRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut target_link_id__ = None;
                let mut text__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TargetLinkId => {
                            if target_link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("targetLinkId"));
                            }
                            target_link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Text => {
                            if text__.is_some() {
                                return Err(serde::de::Error::duplicate_field("text"));
                            }
                            text__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ClientHelloRequest {
                    target_link_id: target_link_id__.unwrap_or_default(),
                    text: text__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientHelloRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientHelloResult {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.target_link_id.is_empty() {
            len += 1;
        }
        if self.round_trip_ms != 0 {
            len += 1;
        }
        if !self.reply.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientHelloResult", len)?;
        if !self.target_link_id.is_empty() {
            struct_ser.serialize_field("targetLinkId", &self.target_link_id)?;
        }
        if self.round_trip_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("roundTripMs", ToString::to_string(&self.round_trip_ms).as_str())?;
        }
        if !self.reply.is_empty() {
            struct_ser.serialize_field("reply", &self.reply)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientHelloResult {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "target_link_id",
            "targetLinkId",
            "round_trip_ms",
            "roundTripMs",
            "reply",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TargetLinkId,
            RoundTripMs,
            Reply,
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
                            "targetLinkId" | "target_link_id" => Ok(GeneratedField::TargetLinkId),
                            "roundTripMs" | "round_trip_ms" => Ok(GeneratedField::RoundTripMs),
                            "reply" => Ok(GeneratedField::Reply),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientHelloResult;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientHelloResult")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientHelloResult, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut target_link_id__ = None;
                let mut round_trip_ms__ = None;
                let mut reply__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TargetLinkId => {
                            if target_link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("targetLinkId"));
                            }
                            target_link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RoundTripMs => {
                            if round_trip_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("roundTripMs"));
                            }
                            round_trip_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Reply => {
                            if reply__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reply"));
                            }
                            reply__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ClientHelloResult {
                    target_link_id: target_link_id__.unwrap_or_default(),
                    round_trip_ms: round_trip_ms__.unwrap_or_default(),
                    reply: reply__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientHelloResult", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientListRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("conex.ClientListRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientListRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientListRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientListRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientListRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(ClientListRequest {
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientListRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientListResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.clients.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientListResponse", len)?;
        if !self.clients.is_empty() {
            struct_ser.serialize_field("clients", &self.clients)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientListResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "clients",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Clients,
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
                            "clients" => Ok(GeneratedField::Clients),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientListResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientListResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientListResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut clients__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Clients => {
                            if clients__.is_some() {
                                return Err(serde::de::Error::duplicate_field("clients"));
                            }
                            clients__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ClientListResponse {
                    clients: clients__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientListResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientProfile {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.display_name.is_empty() {
            len += 1;
        }
        if !self.group.is_empty() {
            len += 1;
        }
        if self.visible {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientProfile", len)?;
        if !self.display_name.is_empty() {
            struct_ser.serialize_field("displayName", &self.display_name)?;
        }
        if !self.group.is_empty() {
            struct_ser.serialize_field("group", &self.group)?;
        }
        if self.visible {
            struct_ser.serialize_field("visible", &self.visible)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientProfile {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "display_name",
            "displayName",
            "group",
            "visible",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            DisplayName,
            Group,
            Visible,
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
                            "displayName" | "display_name" => Ok(GeneratedField::DisplayName),
                            "group" => Ok(GeneratedField::Group),
                            "visible" => Ok(GeneratedField::Visible),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientProfile;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientProfile")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientProfile, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut display_name__ = None;
                let mut group__ = None;
                let mut visible__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::DisplayName => {
                            if display_name__.is_some() {
                                return Err(serde::de::Error::duplicate_field("displayName"));
                            }
                            display_name__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Group => {
                            if group__.is_some() {
                                return Err(serde::de::Error::duplicate_field("group"));
                            }
                            group__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Visible => {
                            if visible__.is_some() {
                                return Err(serde::de::Error::duplicate_field("visible"));
                            }
                            visible__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ClientProfile {
                    display_name: display_name__.unwrap_or_default(),
                    group: group__.unwrap_or_default(),
                    visible: visible__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientProfile", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientProfileRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.profile.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientProfileRequest", len)?;
        if let Some(v) = self.profile.as_ref() {
            struct_ser.serialize_field("profile", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientProfileRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "profile",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Profile,
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
                            "profile" => Ok(GeneratedField::Profile),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientProfileRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientProfileRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientProfileRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut profile__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Profile => {
                            if profile__.is_some() {
                                return Err(serde::de::Error::duplicate_field("profile"));
                            }
                            profile__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ClientProfileRequest {
                    profile: profile__,
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientProfileRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientProfileResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.self_.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientProfileResponse", len)?;
        if let Some(v) = self.self_.as_ref() {
            struct_ser.serialize_field("self", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientProfileResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "self",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Self_,
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
                            "self" => Ok(GeneratedField::Self_),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientProfileResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientProfileResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientProfileResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut self___ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Self_ => {
                            if self___.is_some() {
                                return Err(serde::de::Error::duplicate_field("self"));
                            }
                            self___ = map_.next_value()?;
                        }
                    }
                }
                Ok(ClientProfileResponse {
                    self_: self___,
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientProfileResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientStatusRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("conex.ClientStatusRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientStatusRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientStatusRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientStatusRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientStatusRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(ClientStatusRequest {
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientStatusRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientStatusResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.clients_online != 0 {
            len += 1;
        }
        if self.groups_online != 0 {
            len += 1;
        }
        if !self.groups.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientStatusResponse", len)?;
        if self.clients_online != 0 {
            struct_ser.serialize_field("clientsOnline", &self.clients_online)?;
        }
        if self.groups_online != 0 {
            struct_ser.serialize_field("groupsOnline", &self.groups_online)?;
        }
        if !self.groups.is_empty() {
            struct_ser.serialize_field("groups", &self.groups)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientStatusResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "clients_online",
            "clientsOnline",
            "groups_online",
            "groupsOnline",
            "groups",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ClientsOnline,
            GroupsOnline,
            Groups,
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
                            "clientsOnline" | "clients_online" => Ok(GeneratedField::ClientsOnline),
                            "groupsOnline" | "groups_online" => Ok(GeneratedField::GroupsOnline),
                            "groups" => Ok(GeneratedField::Groups),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientStatusResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientStatusResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientStatusResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut clients_online__ = None;
                let mut groups_online__ = None;
                let mut groups__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ClientsOnline => {
                            if clients_online__.is_some() {
                                return Err(serde::de::Error::duplicate_field("clientsOnline"));
                            }
                            clients_online__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GroupsOnline => {
                            if groups_online__.is_some() {
                                return Err(serde::de::Error::duplicate_field("groupsOnline"));
                            }
                            groups_online__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Groups => {
                            if groups__.is_some() {
                                return Err(serde::de::Error::duplicate_field("groups"));
                            }
                            groups__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ClientStatusResponse {
                    clients_online: clients_online__.unwrap_or_default(),
                    groups_online: groups_online__.unwrap_or_default(),
                    groups: groups__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientStatusResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ClientSummary {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.link_id.is_empty() {
            len += 1;
        }
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if self.profile.is_some() {
            len += 1;
        }
        if self.connected_at_ms != 0 {
            len += 1;
        }
        if self.last_seen_at_ms != 0 {
            len += 1;
        }
        if !self.user_agent.is_empty() {
            len += 1;
        }
        if !self.group_key.is_empty() {
            len += 1;
        }
        if self.last_round_trip_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ClientSummary", len)?;
        if !self.link_id.is_empty() {
            struct_ser.serialize_field("linkId", &self.link_id)?;
        }
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if let Some(v) = self.profile.as_ref() {
            struct_ser.serialize_field("profile", v)?;
        }
        if self.connected_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("connectedAtMs", ToString::to_string(&self.connected_at_ms).as_str())?;
        }
        if self.last_seen_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastSeenAtMs", ToString::to_string(&self.last_seen_at_ms).as_str())?;
        }
        if !self.user_agent.is_empty() {
            struct_ser.serialize_field("userAgent", &self.user_agent)?;
        }
        if !self.group_key.is_empty() {
            struct_ser.serialize_field("groupKey", &self.group_key)?;
        }
        if self.last_round_trip_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastRoundTripMs", ToString::to_string(&self.last_round_trip_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ClientSummary {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "link_id",
            "linkId",
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
            "profile",
            "connected_at_ms",
            "connectedAtMs",
            "last_seen_at_ms",
            "lastSeenAtMs",
            "user_agent",
            "userAgent",
            "group_key",
            "groupKey",
            "last_round_trip_ms",
            "lastRoundTripMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            LinkId,
            PrincipalId,
            TenantId,
            Profile,
            ConnectedAtMs,
            LastSeenAtMs,
            UserAgent,
            GroupKey,
            LastRoundTripMs,
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
                            "linkId" | "link_id" => Ok(GeneratedField::LinkId),
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "profile" => Ok(GeneratedField::Profile),
                            "connectedAtMs" | "connected_at_ms" => Ok(GeneratedField::ConnectedAtMs),
                            "lastSeenAtMs" | "last_seen_at_ms" => Ok(GeneratedField::LastSeenAtMs),
                            "userAgent" | "user_agent" => Ok(GeneratedField::UserAgent),
                            "groupKey" | "group_key" => Ok(GeneratedField::GroupKey),
                            "lastRoundTripMs" | "last_round_trip_ms" => Ok(GeneratedField::LastRoundTripMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ClientSummary;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ClientSummary")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ClientSummary, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut link_id__ = None;
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
                let mut profile__ = None;
                let mut connected_at_ms__ = None;
                let mut last_seen_at_ms__ = None;
                let mut user_agent__ = None;
                let mut group_key__ = None;
                let mut last_round_trip_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::LinkId => {
                            if link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("linkId"));
                            }
                            link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Profile => {
                            if profile__.is_some() {
                                return Err(serde::de::Error::duplicate_field("profile"));
                            }
                            profile__ = map_.next_value()?;
                        }
                        GeneratedField::ConnectedAtMs => {
                            if connected_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("connectedAtMs"));
                            }
                            connected_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastSeenAtMs => {
                            if last_seen_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastSeenAtMs"));
                            }
                            last_seen_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::UserAgent => {
                            if user_agent__.is_some() {
                                return Err(serde::de::Error::duplicate_field("userAgent"));
                            }
                            user_agent__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GroupKey => {
                            if group_key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("groupKey"));
                            }
                            group_key__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LastRoundTripMs => {
                            if last_round_trip_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastRoundTripMs"));
                            }
                            last_round_trip_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(ClientSummary {
                    link_id: link_id__.unwrap_or_default(),
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    profile: profile__,
                    connected_at_ms: connected_at_ms__.unwrap_or_default(),
                    last_seen_at_ms: last_seen_at_ms__.unwrap_or_default(),
                    user_agent: user_agent__.unwrap_or_default(),
                    group_key: group_key__.unwrap_or_default(),
                    last_round_trip_ms: last_round_trip_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ClientSummary", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for CommittedTarget {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.chunk_cid.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.CommittedTarget", len)?;
        if !self.chunk_cid.is_empty() {
            struct_ser.serialize_field("chunkCid", &self.chunk_cid)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for CommittedTarget {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "chunk_cid",
            "chunkCid",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ChunkCid,
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
                            "chunkCid" | "chunk_cid" => Ok(GeneratedField::ChunkCid),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = CommittedTarget;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.CommittedTarget")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<CommittedTarget, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut chunk_cid__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ChunkCid => {
                            if chunk_cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunkCid"));
                            }
                            chunk_cid__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(CommittedTarget {
                    chunk_cid: chunk_cid__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.CommittedTarget", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ConnectionListRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("conex.ConnectionListRequest", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ConnectionListRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ConnectionListRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ConnectionListRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ConnectionListRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(ConnectionListRequest {
                })
            }
        }
        deserializer.deserialize_struct("conex.ConnectionListRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ConnectionListResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.browser_links.is_empty() {
            len += 1;
        }
        if !self.agent_links.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ConnectionListResponse", len)?;
        if !self.browser_links.is_empty() {
            struct_ser.serialize_field("browserLinks", &self.browser_links)?;
        }
        if !self.agent_links.is_empty() {
            struct_ser.serialize_field("agentLinks", &self.agent_links)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ConnectionListResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "browser_links",
            "browserLinks",
            "agent_links",
            "agentLinks",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            BrowserLinks,
            AgentLinks,
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
                            "browserLinks" | "browser_links" => Ok(GeneratedField::BrowserLinks),
                            "agentLinks" | "agent_links" => Ok(GeneratedField::AgentLinks),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ConnectionListResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ConnectionListResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ConnectionListResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut browser_links__ = None;
                let mut agent_links__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::BrowserLinks => {
                            if browser_links__.is_some() {
                                return Err(serde::de::Error::duplicate_field("browserLinks"));
                            }
                            browser_links__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AgentLinks => {
                            if agent_links__.is_some() {
                                return Err(serde::de::Error::duplicate_field("agentLinks"));
                            }
                            agent_links__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ConnectionListResponse {
                    browser_links: browser_links__.unwrap_or_default(),
                    agent_links: agent_links__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ConnectionListResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ConnectionState {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "CONNECTION_STATE_UNSPECIFIED",
            Self::NotApplicable => "CONNECTION_STATE_NOT_APPLICABLE",
            Self::Offline => "CONNECTION_STATE_OFFLINE",
            Self::Ready => "CONNECTION_STATE_READY",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for ConnectionState {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "CONNECTION_STATE_UNSPECIFIED",
            "CONNECTION_STATE_NOT_APPLICABLE",
            "CONNECTION_STATE_OFFLINE",
            "CONNECTION_STATE_READY",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = ConnectionState;

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
                    "CONNECTION_STATE_UNSPECIFIED" => Ok(ConnectionState::Unspecified),
                    "CONNECTION_STATE_NOT_APPLICABLE" => Ok(ConnectionState::NotApplicable),
                    "CONNECTION_STATE_OFFLINE" => Ok(ConnectionState::Offline),
                    "CONNECTION_STATE_READY" => Ok(ConnectionState::Ready),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for ContentAddress {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.address.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ContentAddress", len)?;
        if let Some(v) = self.address.as_ref() {
            match v {
                content_address::Address::RawCid(v) => {
                    struct_ser.serialize_field("rawCid", v)?;
                }
                content_address::Address::ManifestCid(v) => {
                    struct_ser.serialize_field("manifestCid", v)?;
                }
            }
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ContentAddress {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "raw_cid",
            "rawCid",
            "manifest_cid",
            "manifestCid",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RawCid,
            ManifestCid,
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
                            "rawCid" | "raw_cid" => Ok(GeneratedField::RawCid),
                            "manifestCid" | "manifest_cid" => Ok(GeneratedField::ManifestCid),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ContentAddress;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ContentAddress")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ContentAddress, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut address__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RawCid => {
                            if address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rawCid"));
                            }
                            address__ = map_.next_value::<::std::option::Option<_>>()?.map(content_address::Address::RawCid);
                        }
                        GeneratedField::ManifestCid => {
                            if address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("manifestCid"));
                            }
                            address__ = map_.next_value::<::std::option::Option<_>>()?.map(content_address::Address::ManifestCid);
                        }
                    }
                }
                Ok(ContentAddress {
                    address: address__,
                })
            }
        }
        deserializer.deserialize_struct("conex.ContentAddress", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for DataChunk {
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
        if !self.chunk.is_empty() {
            len += 1;
        }
        if !self.revision.is_empty() {
            len += 1;
        }
        if self.eof {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.DataChunk", len)?;
        if !self.request_id.is_empty() {
            struct_ser.serialize_field("requestId", &self.request_id)?;
        }
        if !self.chunk.is_empty() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("chunk", pbjson::private::base64::encode(&self.chunk).as_str())?;
        }
        if !self.revision.is_empty() {
            struct_ser.serialize_field("revision", &self.revision)?;
        }
        if self.eof {
            struct_ser.serialize_field("eof", &self.eof)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for DataChunk {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "request_id",
            "requestId",
            "chunk",
            "revision",
            "eof",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RequestId,
            Chunk,
            Revision,
            Eof,
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
                            "chunk" => Ok(GeneratedField::Chunk),
                            "revision" => Ok(GeneratedField::Revision),
                            "eof" => Ok(GeneratedField::Eof),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = DataChunk;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.DataChunk")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<DataChunk, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut request_id__ = None;
                let mut chunk__ = None;
                let mut revision__ = None;
                let mut eof__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RequestId => {
                            if request_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestId"));
                            }
                            request_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Chunk => {
                            if chunk__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chunk"));
                            }
                            chunk__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Eof => {
                            if eof__.is_some() {
                                return Err(serde::de::Error::duplicate_field("eof"));
                            }
                            eof__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(DataChunk {
                    request_id: request_id__.unwrap_or_default(),
                    chunk: chunk__.unwrap_or_default(),
                    revision: revision__.unwrap_or_default(),
                    eof: eof__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.DataChunk", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for DedupKey {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.provider_endpoint_id.is_empty() {
            len += 1;
        }
        if self.space_id.is_some() {
            len += 1;
        }
        if !self.resource_id.is_empty() {
            len += 1;
        }
        if !self.method.is_empty() {
            len += 1;
        }
        if !self.operation_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.DedupKey", len)?;
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.provider_endpoint_id.is_empty() {
            struct_ser.serialize_field("providerEndpointId", &self.provider_endpoint_id)?;
        }
        if let Some(v) = self.space_id.as_ref() {
            struct_ser.serialize_field("spaceId", v)?;
        }
        if !self.resource_id.is_empty() {
            struct_ser.serialize_field("resourceId", &self.resource_id)?;
        }
        if !self.method.is_empty() {
            struct_ser.serialize_field("method", &self.method)?;
        }
        if !self.operation_id.is_empty() {
            struct_ser.serialize_field("operationId", &self.operation_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for DedupKey {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "tenant_id",
            "tenantId",
            "principal_id",
            "principalId",
            "provider_endpoint_id",
            "providerEndpointId",
            "space_id",
            "spaceId",
            "resource_id",
            "resourceId",
            "method",
            "operation_id",
            "operationId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TenantId,
            PrincipalId,
            ProviderEndpointId,
            SpaceId,
            ResourceId,
            Method,
            OperationId,
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
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "providerEndpointId" | "provider_endpoint_id" => Ok(GeneratedField::ProviderEndpointId),
                            "spaceId" | "space_id" => Ok(GeneratedField::SpaceId),
                            "resourceId" | "resource_id" => Ok(GeneratedField::ResourceId),
                            "method" => Ok(GeneratedField::Method),
                            "operationId" | "operation_id" => Ok(GeneratedField::OperationId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = DedupKey;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.DedupKey")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<DedupKey, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut tenant_id__ = None;
                let mut principal_id__ = None;
                let mut provider_endpoint_id__ = None;
                let mut space_id__ = None;
                let mut resource_id__ = None;
                let mut method__ = None;
                let mut operation_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ProviderEndpointId => {
                            if provider_endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("providerEndpointId"));
                            }
                            provider_endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::SpaceId => {
                            if space_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("spaceId"));
                            }
                            space_id__ = map_.next_value()?;
                        }
                        GeneratedField::ResourceId => {
                            if resource_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceId"));
                            }
                            resource_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Method => {
                            if method__.is_some() {
                                return Err(serde::de::Error::duplicate_field("method"));
                            }
                            method__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OperationId => {
                            if operation_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("operationId"));
                            }
                            operation_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(DedupKey {
                    tenant_id: tenant_id__.unwrap_or_default(),
                    principal_id: principal_id__.unwrap_or_default(),
                    provider_endpoint_id: provider_endpoint_id__.unwrap_or_default(),
                    space_id: space_id__,
                    resource_id: resource_id__.unwrap_or_default(),
                    method: method__.unwrap_or_default(),
                    operation_id: operation_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.DedupKey", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EndpointListRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.limit.is_some() {
            len += 1;
        }
        if self.after_endpoint_id.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.EndpointListRequest", len)?;
        if let Some(v) = self.limit.as_ref() {
            struct_ser.serialize_field("limit", v)?;
        }
        if let Some(v) = self.after_endpoint_id.as_ref() {
            struct_ser.serialize_field("afterEndpointId", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EndpointListRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "limit",
            "after_endpoint_id",
            "afterEndpointId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Limit,
            AfterEndpointId,
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
                            "limit" => Ok(GeneratedField::Limit),
                            "afterEndpointId" | "after_endpoint_id" => Ok(GeneratedField::AfterEndpointId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EndpointListRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.EndpointListRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EndpointListRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut limit__ = None;
                let mut after_endpoint_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Limit => {
                            if limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limit"));
                            }
                            limit__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::AfterEndpointId => {
                            if after_endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("afterEndpointId"));
                            }
                            after_endpoint_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(EndpointListRequest {
                    limit: limit__,
                    after_endpoint_id: after_endpoint_id__,
                })
            }
        }
        deserializer.deserialize_struct("conex.EndpointListRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EndpointListResult {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.endpoints.is_empty() {
            len += 1;
        }
        if self.next_after_endpoint_id.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.EndpointListResult", len)?;
        if !self.endpoints.is_empty() {
            struct_ser.serialize_field("endpoints", &self.endpoints)?;
        }
        if let Some(v) = self.next_after_endpoint_id.as_ref() {
            struct_ser.serialize_field("nextAfterEndpointId", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EndpointListResult {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "endpoints",
            "next_after_endpoint_id",
            "nextAfterEndpointId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Endpoints,
            NextAfterEndpointId,
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
                            "endpoints" => Ok(GeneratedField::Endpoints),
                            "nextAfterEndpointId" | "next_after_endpoint_id" => Ok(GeneratedField::NextAfterEndpointId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EndpointListResult;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.EndpointListResult")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EndpointListResult, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut endpoints__ = None;
                let mut next_after_endpoint_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Endpoints => {
                            if endpoints__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpoints"));
                            }
                            endpoints__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NextAfterEndpointId => {
                            if next_after_endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("nextAfterEndpointId"));
                            }
                            next_after_endpoint_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(EndpointListResult {
                    endpoints: endpoints__.unwrap_or_default(),
                    next_after_endpoint_id: next_after_endpoint_id__,
                })
            }
        }
        deserializer.deserialize_struct("conex.EndpointListResult", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EndpointSummary {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.endpoint_id.is_empty() {
            len += 1;
        }
        if !self.display_name.is_empty() {
            len += 1;
        }
        if !self.provider_id.is_empty() {
            len += 1;
        }
        if self.attachment_kind != 0 {
            len += 1;
        }
        if self.agent_id.is_some() {
            len += 1;
        }
        if self.region.is_some() {
            len += 1;
        }
        if self.connection_state != 0 {
            len += 1;
        }
        if !self.available_methods.is_empty() {
            len += 1;
        }
        if !self.authorized_scopes.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.EndpointSummary", len)?;
        if !self.endpoint_id.is_empty() {
            struct_ser.serialize_field("endpointId", &self.endpoint_id)?;
        }
        if !self.display_name.is_empty() {
            struct_ser.serialize_field("displayName", &self.display_name)?;
        }
        if !self.provider_id.is_empty() {
            struct_ser.serialize_field("providerId", &self.provider_id)?;
        }
        if self.attachment_kind != 0 {
            let v = AttachmentKind::try_from(self.attachment_kind)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.attachment_kind)))?;
            struct_ser.serialize_field("attachmentKind", &v)?;
        }
        if let Some(v) = self.agent_id.as_ref() {
            struct_ser.serialize_field("agentId", v)?;
        }
        if let Some(v) = self.region.as_ref() {
            struct_ser.serialize_field("region", v)?;
        }
        if self.connection_state != 0 {
            let v = ConnectionState::try_from(self.connection_state)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.connection_state)))?;
            struct_ser.serialize_field("connectionState", &v)?;
        }
        if !self.available_methods.is_empty() {
            struct_ser.serialize_field("availableMethods", &self.available_methods)?;
        }
        if !self.authorized_scopes.is_empty() {
            struct_ser.serialize_field("authorizedScopes", &self.authorized_scopes)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EndpointSummary {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "endpoint_id",
            "endpointId",
            "display_name",
            "displayName",
            "provider_id",
            "providerId",
            "attachment_kind",
            "attachmentKind",
            "agent_id",
            "agentId",
            "region",
            "connection_state",
            "connectionState",
            "available_methods",
            "availableMethods",
            "authorized_scopes",
            "authorizedScopes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EndpointId,
            DisplayName,
            ProviderId,
            AttachmentKind,
            AgentId,
            Region,
            ConnectionState,
            AvailableMethods,
            AuthorizedScopes,
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
                            "endpointId" | "endpoint_id" => Ok(GeneratedField::EndpointId),
                            "displayName" | "display_name" => Ok(GeneratedField::DisplayName),
                            "providerId" | "provider_id" => Ok(GeneratedField::ProviderId),
                            "attachmentKind" | "attachment_kind" => Ok(GeneratedField::AttachmentKind),
                            "agentId" | "agent_id" => Ok(GeneratedField::AgentId),
                            "region" => Ok(GeneratedField::Region),
                            "connectionState" | "connection_state" => Ok(GeneratedField::ConnectionState),
                            "availableMethods" | "available_methods" => Ok(GeneratedField::AvailableMethods),
                            "authorizedScopes" | "authorized_scopes" => Ok(GeneratedField::AuthorizedScopes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EndpointSummary;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.EndpointSummary")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EndpointSummary, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut endpoint_id__ = None;
                let mut display_name__ = None;
                let mut provider_id__ = None;
                let mut attachment_kind__ = None;
                let mut agent_id__ = None;
                let mut region__ = None;
                let mut connection_state__ = None;
                let mut available_methods__ = None;
                let mut authorized_scopes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EndpointId => {
                            if endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointId"));
                            }
                            endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DisplayName => {
                            if display_name__.is_some() {
                                return Err(serde::de::Error::duplicate_field("displayName"));
                            }
                            display_name__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ProviderId => {
                            if provider_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("providerId"));
                            }
                            provider_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentKind => {
                            if attachment_kind__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentKind"));
                            }
                            attachment_kind__ = Some(map_.next_value::<AttachmentKind>()? as i32);
                        }
                        GeneratedField::AgentId => {
                            if agent_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("agentId"));
                            }
                            agent_id__ = map_.next_value()?;
                        }
                        GeneratedField::Region => {
                            if region__.is_some() {
                                return Err(serde::de::Error::duplicate_field("region"));
                            }
                            region__ = map_.next_value()?;
                        }
                        GeneratedField::ConnectionState => {
                            if connection_state__.is_some() {
                                return Err(serde::de::Error::duplicate_field("connectionState"));
                            }
                            connection_state__ = Some(map_.next_value::<ConnectionState>()? as i32);
                        }
                        GeneratedField::AvailableMethods => {
                            if available_methods__.is_some() {
                                return Err(serde::de::Error::duplicate_field("availableMethods"));
                            }
                            available_methods__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AuthorizedScopes => {
                            if authorized_scopes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("authorizedScopes"));
                            }
                            authorized_scopes__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EndpointSummary {
                    endpoint_id: endpoint_id__.unwrap_or_default(),
                    display_name: display_name__.unwrap_or_default(),
                    provider_id: provider_id__.unwrap_or_default(),
                    attachment_kind: attachment_kind__.unwrap_or_default(),
                    agent_id: agent_id__,
                    region: region__,
                    connection_state: connection_state__.unwrap_or_default(),
                    available_methods: available_methods__.unwrap_or_default(),
                    authorized_scopes: authorized_scopes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.EndpointSummary", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EntryKind {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "ENTRY_KIND_UNSPECIFIED",
            Self::File => "ENTRY_KIND_FILE",
            Self::Directory => "ENTRY_KIND_DIRECTORY",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for EntryKind {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ENTRY_KIND_UNSPECIFIED",
            "ENTRY_KIND_FILE",
            "ENTRY_KIND_DIRECTORY",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = EntryKind;

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
                    "ENTRY_KIND_UNSPECIFIED" => Ok(EntryKind::Unspecified),
                    "ENTRY_KIND_FILE" => Ok(EntryKind::File),
                    "ENTRY_KIND_DIRECTORY" => Ok(EntryKind::Directory),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Error", len)?;
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
                formatter.write_str("struct conex.Error")
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
        deserializer.deserialize_struct("conex.Error", FIELDS, GeneratedVisitor)
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
impl serde::Serialize for ExecutionClass {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "EXECUTION_CLASS_UNSPECIFIED",
            Self::ReadOnly => "EXECUTION_CLASS_READ_ONLY",
            Self::Idempotent => "EXECUTION_CLASS_IDEMPOTENT",
            Self::Deduplicated => "EXECUTION_CLASS_DEDUPLICATED",
            Self::NonReplayable => "EXECUTION_CLASS_NON_REPLAYABLE",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for ExecutionClass {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "EXECUTION_CLASS_UNSPECIFIED",
            "EXECUTION_CLASS_READ_ONLY",
            "EXECUTION_CLASS_IDEMPOTENT",
            "EXECUTION_CLASS_DEDUPLICATED",
            "EXECUTION_CLASS_NON_REPLAYABLE",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = ExecutionClass;

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
                    "EXECUTION_CLASS_UNSPECIFIED" => Ok(ExecutionClass::Unspecified),
                    "EXECUTION_CLASS_READ_ONLY" => Ok(ExecutionClass::ReadOnly),
                    "EXECUTION_CLASS_IDEMPOTENT" => Ok(ExecutionClass::Idempotent),
                    "EXECUTION_CLASS_DEDUPLICATED" => Ok(ExecutionClass::Deduplicated),
                    "EXECUTION_CLASS_NON_REPLAYABLE" => Ok(ExecutionClass::NonReplayable),
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
        let mut struct_ser = serializer.serialize_struct("conex.Failure", len)?;
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
                formatter.write_str("struct conex.Failure")
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
        deserializer.deserialize_struct("conex.Failure", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GroupStatus {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.group_key.is_empty() {
            len += 1;
        }
        if !self.label.is_empty() {
            len += 1;
        }
        if self.clients_online != 0 {
            len += 1;
        }
        if self.last_round_trip_ms != 0 {
            len += 1;
        }
        if self.avg_round_trip_ms != 0 {
            len += 1;
        }
        if self.round_trips != 0 {
            len += 1;
        }
        if self.files_shared != 0 {
            len += 1;
        }
        if self.shared_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.GroupStatus", len)?;
        if !self.group_key.is_empty() {
            struct_ser.serialize_field("groupKey", &self.group_key)?;
        }
        if !self.label.is_empty() {
            struct_ser.serialize_field("label", &self.label)?;
        }
        if self.clients_online != 0 {
            struct_ser.serialize_field("clientsOnline", &self.clients_online)?;
        }
        if self.last_round_trip_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastRoundTripMs", ToString::to_string(&self.last_round_trip_ms).as_str())?;
        }
        if self.avg_round_trip_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("avgRoundTripMs", ToString::to_string(&self.avg_round_trip_ms).as_str())?;
        }
        if self.round_trips != 0 {
            struct_ser.serialize_field("roundTrips", &self.round_trips)?;
        }
        if self.files_shared != 0 {
            struct_ser.serialize_field("filesShared", &self.files_shared)?;
        }
        if self.shared_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("sharedBytes", ToString::to_string(&self.shared_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GroupStatus {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "group_key",
            "groupKey",
            "label",
            "clients_online",
            "clientsOnline",
            "last_round_trip_ms",
            "lastRoundTripMs",
            "avg_round_trip_ms",
            "avgRoundTripMs",
            "round_trips",
            "roundTrips",
            "files_shared",
            "filesShared",
            "shared_bytes",
            "sharedBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            GroupKey,
            Label,
            ClientsOnline,
            LastRoundTripMs,
            AvgRoundTripMs,
            RoundTrips,
            FilesShared,
            SharedBytes,
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
                            "groupKey" | "group_key" => Ok(GeneratedField::GroupKey),
                            "label" => Ok(GeneratedField::Label),
                            "clientsOnline" | "clients_online" => Ok(GeneratedField::ClientsOnline),
                            "lastRoundTripMs" | "last_round_trip_ms" => Ok(GeneratedField::LastRoundTripMs),
                            "avgRoundTripMs" | "avg_round_trip_ms" => Ok(GeneratedField::AvgRoundTripMs),
                            "roundTrips" | "round_trips" => Ok(GeneratedField::RoundTrips),
                            "filesShared" | "files_shared" => Ok(GeneratedField::FilesShared),
                            "sharedBytes" | "shared_bytes" => Ok(GeneratedField::SharedBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GroupStatus;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.GroupStatus")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GroupStatus, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut group_key__ = None;
                let mut label__ = None;
                let mut clients_online__ = None;
                let mut last_round_trip_ms__ = None;
                let mut avg_round_trip_ms__ = None;
                let mut round_trips__ = None;
                let mut files_shared__ = None;
                let mut shared_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::GroupKey => {
                            if group_key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("groupKey"));
                            }
                            group_key__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Label => {
                            if label__.is_some() {
                                return Err(serde::de::Error::duplicate_field("label"));
                            }
                            label__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ClientsOnline => {
                            if clients_online__.is_some() {
                                return Err(serde::de::Error::duplicate_field("clientsOnline"));
                            }
                            clients_online__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastRoundTripMs => {
                            if last_round_trip_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastRoundTripMs"));
                            }
                            last_round_trip_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::AvgRoundTripMs => {
                            if avg_round_trip_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("avgRoundTripMs"));
                            }
                            avg_round_trip_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RoundTrips => {
                            if round_trips__.is_some() {
                                return Err(serde::de::Error::duplicate_field("roundTrips"));
                            }
                            round_trips__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::FilesShared => {
                            if files_shared__.is_some() {
                                return Err(serde::de::Error::duplicate_field("filesShared"));
                            }
                            files_shared__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::SharedBytes => {
                            if shared_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sharedBytes"));
                            }
                            shared_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(GroupStatus {
                    group_key: group_key__.unwrap_or_default(),
                    label: label__.unwrap_or_default(),
                    clients_online: clients_online__.unwrap_or_default(),
                    last_round_trip_ms: last_round_trip_ms__.unwrap_or_default(),
                    avg_round_trip_ms: avg_round_trip_ms__.unwrap_or_default(),
                    round_trips: round_trips__.unwrap_or_default(),
                    files_shared: files_shared__.unwrap_or_default(),
                    shared_bytes: shared_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.GroupStatus", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.HelloRequest", len)?;
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
                formatter.write_str("struct conex.HelloRequest")
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
        deserializer.deserialize_struct("conex.HelloRequest", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.HelloResponse", len)?;
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
                formatter.write_str("struct conex.HelloResponse")
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
        deserializer.deserialize_struct("conex.HelloResponse", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Limits", len)?;
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
                formatter.write_str("struct conex.Limits")
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
        deserializer.deserialize_struct("conex.Limits", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for LinkIdentity {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.link_id.is_empty() {
            len += 1;
        }
        if !self.peer_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.LinkIdentity", len)?;
        if !self.link_id.is_empty() {
            struct_ser.serialize_field("linkId", &self.link_id)?;
        }
        if !self.peer_id.is_empty() {
            struct_ser.serialize_field("peerId", &self.peer_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for LinkIdentity {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "link_id",
            "linkId",
            "peer_id",
            "peerId",
            "tenant_id",
            "tenantId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            LinkId,
            PeerId,
            TenantId,
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
                            "linkId" | "link_id" => Ok(GeneratedField::LinkId),
                            "peerId" | "peer_id" => Ok(GeneratedField::PeerId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = LinkIdentity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.LinkIdentity")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<LinkIdentity, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut link_id__ = None;
                let mut peer_id__ = None;
                let mut tenant_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::LinkId => {
                            if link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("linkId"));
                            }
                            link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PeerId => {
                            if peer_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("peerId"));
                            }
                            peer_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(LinkIdentity {
                    link_id: link_id__.unwrap_or_default(),
                    peer_id: peer_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.LinkIdentity", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Manifest", len)?;
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
                formatter.write_str("struct conex.Manifest")
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
        deserializer.deserialize_struct("conex.Manifest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ManifestEntries {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.leaves.is_empty() {
            len += 1;
        }
        if !self.child_manifest_cids.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ManifestEntries", len)?;
        if !self.leaves.is_empty() {
            struct_ser.serialize_field("leaves", &self.leaves)?;
        }
        if !self.child_manifest_cids.is_empty() {
            struct_ser.serialize_field("childManifestCids", &self.child_manifest_cids)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ManifestEntries {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "leaves",
            "child_manifest_cids",
            "childManifestCids",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Leaves,
            ChildManifestCids,
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
                            "leaves" => Ok(GeneratedField::Leaves),
                            "childManifestCids" | "child_manifest_cids" => Ok(GeneratedField::ChildManifestCids),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ManifestEntries;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ManifestEntries")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ManifestEntries, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut leaves__ = None;
                let mut child_manifest_cids__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Leaves => {
                            if leaves__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leaves"));
                            }
                            leaves__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ChildManifestCids => {
                            if child_manifest_cids__.is_some() {
                                return Err(serde::de::Error::duplicate_field("childManifestCids"));
                            }
                            child_manifest_cids__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ManifestEntries {
                    leaves: leaves__.unwrap_or_default(),
                    child_manifest_cids: child_manifest_cids__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ManifestEntries", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Message", len)?;
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
                message::Body::DataChunk(v) => {
                    struct_ser.serialize_field("dataChunk", v)?;
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
            "data_chunk",
            "dataChunk",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Request,
            Success,
            Failure,
            Notification,
            DataChunk,
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
                            "dataChunk" | "data_chunk" => Ok(GeneratedField::DataChunk),
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
                formatter.write_str("struct conex.Message")
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
                        GeneratedField::DataChunk => {
                            if body__.is_some() {
                                return Err(serde::de::Error::duplicate_field("dataChunk"));
                            }
                            body__ = map_.next_value::<::std::option::Option<_>>()?.map(message::Body::DataChunk)
;
                        }
                    }
                }
                Ok(Message {
                    body: body__,
                })
            }
        }
        deserializer.deserialize_struct("conex.Message", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for NegotiatedCapabilities {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.requires.is_empty() {
            len += 1;
        }
        if !self.rejected_capabilities.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.NegotiatedCapabilities", len)?;
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if !self.requires.is_empty() {
            struct_ser.serialize_field("requires", &self.requires)?;
        }
        if !self.rejected_capabilities.is_empty() {
            struct_ser.serialize_field("rejectedCapabilities", &self.rejected_capabilities)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NegotiatedCapabilities {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "provides",
            "requires",
            "rejected_capabilities",
            "rejectedCapabilities",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Provides,
            Requires,
            RejectedCapabilities,
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
                            "provides" => Ok(GeneratedField::Provides),
                            "requires" => Ok(GeneratedField::Requires),
                            "rejectedCapabilities" | "rejected_capabilities" => Ok(GeneratedField::RejectedCapabilities),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = NegotiatedCapabilities;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.NegotiatedCapabilities")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NegotiatedCapabilities, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut provides__ = None;
                let mut requires__ = None;
                let mut rejected_capabilities__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
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
                        GeneratedField::RejectedCapabilities => {
                            if rejected_capabilities__.is_some() {
                                return Err(serde::de::Error::duplicate_field("rejectedCapabilities"));
                            }
                            rejected_capabilities__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(NegotiatedCapabilities {
                    provides: provides__.unwrap_or_default(),
                    requires: requires__.unwrap_or_default(),
                    rejected_capabilities: rejected_capabilities__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.NegotiatedCapabilities", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Notification", len)?;
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
                formatter.write_str("struct conex.Notification")
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
        deserializer.deserialize_struct("conex.Notification", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationCancelRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.key.is_some() {
            len += 1;
        }
        if !self.reason.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.OperationCancelRequest", len)?;
        if let Some(v) = self.key.as_ref() {
            struct_ser.serialize_field("key", v)?;
        }
        if !self.reason.is_empty() {
            struct_ser.serialize_field("reason", &self.reason)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for OperationCancelRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "key",
            "reason",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Key,
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
                            "key" => Ok(GeneratedField::Key),
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
            type Value = OperationCancelRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.OperationCancelRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<OperationCancelRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut key__ = None;
                let mut reason__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Key => {
                            if key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("key"));
                            }
                            key__ = map_.next_value()?;
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(OperationCancelRequest {
                    key: key__,
                    reason: reason__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.OperationCancelRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationCancelResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.state != 0 {
            len += 1;
        }
        if self.outcome_unknown {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.OperationCancelResponse", len)?;
        if self.state != 0 {
            let v = OperationState::try_from(self.state)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.state)))?;
            struct_ser.serialize_field("state", &v)?;
        }
        if self.outcome_unknown {
            struct_ser.serialize_field("outcomeUnknown", &self.outcome_unknown)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for OperationCancelResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "state",
            "outcome_unknown",
            "outcomeUnknown",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            State,
            OutcomeUnknown,
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
                            "state" => Ok(GeneratedField::State),
                            "outcomeUnknown" | "outcome_unknown" => Ok(GeneratedField::OutcomeUnknown),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = OperationCancelResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.OperationCancelResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<OperationCancelResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut state__ = None;
                let mut outcome_unknown__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::State => {
                            if state__.is_some() {
                                return Err(serde::de::Error::duplicate_field("state"));
                            }
                            state__ = Some(map_.next_value::<OperationState>()? as i32);
                        }
                        GeneratedField::OutcomeUnknown => {
                            if outcome_unknown__.is_some() {
                                return Err(serde::de::Error::duplicate_field("outcomeUnknown"));
                            }
                            outcome_unknown__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(OperationCancelResponse {
                    state: state__.unwrap_or_default(),
                    outcome_unknown: outcome_unknown__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.OperationCancelResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationGetRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.key.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.OperationGetRequest", len)?;
        if let Some(v) = self.key.as_ref() {
            struct_ser.serialize_field("key", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for OperationGetRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "key",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Key,
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
                            "key" => Ok(GeneratedField::Key),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = OperationGetRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.OperationGetRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<OperationGetRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut key__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Key => {
                            if key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("key"));
                            }
                            key__ = map_.next_value()?;
                        }
                    }
                }
                Ok(OperationGetRequest {
                    key: key__,
                })
            }
        }
        deserializer.deserialize_struct("conex.OperationGetRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationGetResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.key.is_some() {
            len += 1;
        }
        if self.state != 0 {
            len += 1;
        }
        if self.expires_at_ms != 0 {
            len += 1;
        }
        if self.success.is_some() {
            len += 1;
        }
        if self.failure.is_some() {
            len += 1;
        }
        if !self.execution.is_empty() {
            len += 1;
        }
        if self.accepted_at_ms != 0 {
            len += 1;
        }
        if self.settled_at_ms.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.OperationGetResponse", len)?;
        if let Some(v) = self.key.as_ref() {
            struct_ser.serialize_field("key", v)?;
        }
        if self.state != 0 {
            let v = OperationState::try_from(self.state)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.state)))?;
            struct_ser.serialize_field("state", &v)?;
        }
        if self.expires_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expiresAtMs", ToString::to_string(&self.expires_at_ms).as_str())?;
        }
        if let Some(v) = self.success.as_ref() {
            struct_ser.serialize_field("success", v)?;
        }
        if let Some(v) = self.failure.as_ref() {
            struct_ser.serialize_field("failure", v)?;
        }
        if !self.execution.is_empty() {
            struct_ser.serialize_field("execution", &self.execution)?;
        }
        if self.accepted_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("acceptedAtMs", ToString::to_string(&self.accepted_at_ms).as_str())?;
        }
        if let Some(v) = self.settled_at_ms.as_ref() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("settledAtMs", ToString::to_string(&v).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for OperationGetResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "key",
            "state",
            "expires_at_ms",
            "expiresAtMs",
            "success",
            "failure",
            "execution",
            "accepted_at_ms",
            "acceptedAtMs",
            "settled_at_ms",
            "settledAtMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Key,
            State,
            ExpiresAtMs,
            Success,
            Failure,
            Execution,
            AcceptedAtMs,
            SettledAtMs,
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
                            "key" => Ok(GeneratedField::Key),
                            "state" => Ok(GeneratedField::State),
                            "expiresAtMs" | "expires_at_ms" => Ok(GeneratedField::ExpiresAtMs),
                            "success" => Ok(GeneratedField::Success),
                            "failure" => Ok(GeneratedField::Failure),
                            "execution" => Ok(GeneratedField::Execution),
                            "acceptedAtMs" | "accepted_at_ms" => Ok(GeneratedField::AcceptedAtMs),
                            "settledAtMs" | "settled_at_ms" => Ok(GeneratedField::SettledAtMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = OperationGetResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.OperationGetResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<OperationGetResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut key__ = None;
                let mut state__ = None;
                let mut expires_at_ms__ = None;
                let mut success__ = None;
                let mut failure__ = None;
                let mut execution__ = None;
                let mut accepted_at_ms__ = None;
                let mut settled_at_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Key => {
                            if key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("key"));
                            }
                            key__ = map_.next_value()?;
                        }
                        GeneratedField::State => {
                            if state__.is_some() {
                                return Err(serde::de::Error::duplicate_field("state"));
                            }
                            state__ = Some(map_.next_value::<OperationState>()? as i32);
                        }
                        GeneratedField::ExpiresAtMs => {
                            if expires_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expiresAtMs"));
                            }
                            expires_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Success => {
                            if success__.is_some() {
                                return Err(serde::de::Error::duplicate_field("success"));
                            }
                            success__ = map_.next_value()?;
                        }
                        GeneratedField::Failure => {
                            if failure__.is_some() {
                                return Err(serde::de::Error::duplicate_field("failure"));
                            }
                            failure__ = map_.next_value()?;
                        }
                        GeneratedField::Execution => {
                            if execution__.is_some() {
                                return Err(serde::de::Error::duplicate_field("execution"));
                            }
                            execution__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AcceptedAtMs => {
                            if accepted_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("acceptedAtMs"));
                            }
                            accepted_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::SettledAtMs => {
                            if settled_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("settledAtMs"));
                            }
                            settled_at_ms__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                    }
                }
                Ok(OperationGetResponse {
                    key: key__,
                    state: state__.unwrap_or_default(),
                    expires_at_ms: expires_at_ms__.unwrap_or_default(),
                    success: success__,
                    failure: failure__,
                    execution: execution__.unwrap_or_default(),
                    accepted_at_ms: accepted_at_ms__.unwrap_or_default(),
                    settled_at_ms: settled_at_ms__,
                })
            }
        }
        deserializer.deserialize_struct("conex.OperationGetResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationRecord {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.schema_version != 0 {
            len += 1;
        }
        if self.key.is_some() {
            len += 1;
        }
        if self.state != 0 {
            len += 1;
        }
        if self.execution_class != 0 {
            len += 1;
        }
        if !self.param_digest.is_empty() {
            len += 1;
        }
        if self.accepted_at_ms != 0 {
            len += 1;
        }
        if self.settled_at_ms.is_some() {
            len += 1;
        }
        if self.expires_at_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.OperationRecord", len)?;
        if self.schema_version != 0 {
            struct_ser.serialize_field("schemaVersion", &self.schema_version)?;
        }
        if let Some(v) = self.key.as_ref() {
            struct_ser.serialize_field("key", v)?;
        }
        if self.state != 0 {
            let v = OperationState::try_from(self.state)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.state)))?;
            struct_ser.serialize_field("state", &v)?;
        }
        if self.execution_class != 0 {
            let v = ExecutionClass::try_from(self.execution_class)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.execution_class)))?;
            struct_ser.serialize_field("executionClass", &v)?;
        }
        if !self.param_digest.is_empty() {
            struct_ser.serialize_field("paramDigest", &self.param_digest)?;
        }
        if self.accepted_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("acceptedAtMs", ToString::to_string(&self.accepted_at_ms).as_str())?;
        }
        if let Some(v) = self.settled_at_ms.as_ref() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("settledAtMs", ToString::to_string(&v).as_str())?;
        }
        if self.expires_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expiresAtMs", ToString::to_string(&self.expires_at_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for OperationRecord {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "schema_version",
            "schemaVersion",
            "key",
            "state",
            "execution_class",
            "executionClass",
            "param_digest",
            "paramDigest",
            "accepted_at_ms",
            "acceptedAtMs",
            "settled_at_ms",
            "settledAtMs",
            "expires_at_ms",
            "expiresAtMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SchemaVersion,
            Key,
            State,
            ExecutionClass,
            ParamDigest,
            AcceptedAtMs,
            SettledAtMs,
            ExpiresAtMs,
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
                            "schemaVersion" | "schema_version" => Ok(GeneratedField::SchemaVersion),
                            "key" => Ok(GeneratedField::Key),
                            "state" => Ok(GeneratedField::State),
                            "executionClass" | "execution_class" => Ok(GeneratedField::ExecutionClass),
                            "paramDigest" | "param_digest" => Ok(GeneratedField::ParamDigest),
                            "acceptedAtMs" | "accepted_at_ms" => Ok(GeneratedField::AcceptedAtMs),
                            "settledAtMs" | "settled_at_ms" => Ok(GeneratedField::SettledAtMs),
                            "expiresAtMs" | "expires_at_ms" => Ok(GeneratedField::ExpiresAtMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = OperationRecord;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.OperationRecord")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<OperationRecord, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut schema_version__ = None;
                let mut key__ = None;
                let mut state__ = None;
                let mut execution_class__ = None;
                let mut param_digest__ = None;
                let mut accepted_at_ms__ = None;
                let mut settled_at_ms__ = None;
                let mut expires_at_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SchemaVersion => {
                            if schema_version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("schemaVersion"));
                            }
                            schema_version__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Key => {
                            if key__.is_some() {
                                return Err(serde::de::Error::duplicate_field("key"));
                            }
                            key__ = map_.next_value()?;
                        }
                        GeneratedField::State => {
                            if state__.is_some() {
                                return Err(serde::de::Error::duplicate_field("state"));
                            }
                            state__ = Some(map_.next_value::<OperationState>()? as i32);
                        }
                        GeneratedField::ExecutionClass => {
                            if execution_class__.is_some() {
                                return Err(serde::de::Error::duplicate_field("executionClass"));
                            }
                            execution_class__ = Some(map_.next_value::<ExecutionClass>()? as i32);
                        }
                        GeneratedField::ParamDigest => {
                            if param_digest__.is_some() {
                                return Err(serde::de::Error::duplicate_field("paramDigest"));
                            }
                            param_digest__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AcceptedAtMs => {
                            if accepted_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("acceptedAtMs"));
                            }
                            accepted_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::SettledAtMs => {
                            if settled_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("settledAtMs"));
                            }
                            settled_at_ms__ = 
                                map_.next_value::<::std::option::Option<::pbjson::private::NumberDeserialize<_>>>()?.map(|x| x.0)
                            ;
                        }
                        GeneratedField::ExpiresAtMs => {
                            if expires_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expiresAtMs"));
                            }
                            expires_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(OperationRecord {
                    schema_version: schema_version__.unwrap_or_default(),
                    key: key__,
                    state: state__.unwrap_or_default(),
                    execution_class: execution_class__.unwrap_or_default(),
                    param_digest: param_digest__.unwrap_or_default(),
                    accepted_at_ms: accepted_at_ms__.unwrap_or_default(),
                    settled_at_ms: settled_at_ms__,
                    expires_at_ms: expires_at_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.OperationRecord", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for OperationState {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "OPERATION_STATE_UNSPECIFIED",
            Self::Accepted => "OPERATION_STATE_ACCEPTED",
            Self::Running => "OPERATION_STATE_RUNNING",
            Self::Succeeded => "OPERATION_STATE_SUCCEEDED",
            Self::Failed => "OPERATION_STATE_FAILED",
            Self::Cancelled => "OPERATION_STATE_CANCELLED",
            Self::Unknown => "OPERATION_STATE_UNKNOWN",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for OperationState {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "OPERATION_STATE_UNSPECIFIED",
            "OPERATION_STATE_ACCEPTED",
            "OPERATION_STATE_RUNNING",
            "OPERATION_STATE_SUCCEEDED",
            "OPERATION_STATE_FAILED",
            "OPERATION_STATE_CANCELLED",
            "OPERATION_STATE_UNKNOWN",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = OperationState;

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
                    "OPERATION_STATE_UNSPECIFIED" => Ok(OperationState::Unspecified),
                    "OPERATION_STATE_ACCEPTED" => Ok(OperationState::Accepted),
                    "OPERATION_STATE_RUNNING" => Ok(OperationState::Running),
                    "OPERATION_STATE_SUCCEEDED" => Ok(OperationState::Succeeded),
                    "OPERATION_STATE_FAILED" => Ok(OperationState::Failed),
                    "OPERATION_STATE_CANCELLED" => Ok(OperationState::Cancelled),
                    "OPERATION_STATE_UNKNOWN" => Ok(OperationState::Unknown),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for PersistenceLevel {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "PERSISTENCE_LEVEL_UNSPECIFIED",
            Self::Local => "PERSISTENCE_LEVEL_LOCAL",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for PersistenceLevel {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "PERSISTENCE_LEVEL_UNSPECIFIED",
            "PERSISTENCE_LEVEL_LOCAL",
        ];

        struct GeneratedVisitor;

        impl serde::de::Visitor<'_> for GeneratedVisitor {
            type Value = PersistenceLevel;

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
                    "PERSISTENCE_LEVEL_UNSPECIFIED" => Ok(PersistenceLevel::Unspecified),
                    "PERSISTENCE_LEVEL_LOCAL" => Ok(PersistenceLevel::Local),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
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
impl serde::Serialize for ReadyRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.negotiation_id.is_empty() {
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
        if self.limits.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ReadyRequest", len)?;
        if !self.negotiation_id.is_empty() {
            struct_ser.serialize_field("negotiationId", &self.negotiation_id)?;
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
        if let Some(v) = self.limits.as_ref() {
            struct_ser.serialize_field("limits", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ReadyRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "negotiation_id",
            "negotiationId",
            "profile_id",
            "profileId",
            "plane",
            "provides",
            "limits",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NegotiationId,
            ProfileId,
            Plane,
            Provides,
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
                            "negotiationId" | "negotiation_id" => Ok(GeneratedField::NegotiationId),
                            "profileId" | "profile_id" => Ok(GeneratedField::ProfileId),
                            "plane" => Ok(GeneratedField::Plane),
                            "provides" => Ok(GeneratedField::Provides),
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
            type Value = ReadyRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ReadyRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ReadyRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut negotiation_id__ = None;
                let mut profile_id__ = None;
                let mut plane__ = None;
                let mut provides__ = None;
                let mut limits__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NegotiationId => {
                            if negotiation_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("negotiationId"));
                            }
                            negotiation_id__ = Some(map_.next_value()?);
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
                        GeneratedField::Limits => {
                            if limits__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limits"));
                            }
                            limits__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ReadyRequest {
                    negotiation_id: negotiation_id__.unwrap_or_default(),
                    profile_id: profile_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    limits: limits__,
                })
            }
        }
        deserializer.deserialize_struct("conex.ReadyRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ReadyResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.negotiation_id.is_empty() {
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
        if self.limits.is_some() {
            len += 1;
        }
        if self.link_identity.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ReadyResponse", len)?;
        if !self.negotiation_id.is_empty() {
            struct_ser.serialize_field("negotiationId", &self.negotiation_id)?;
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
        if let Some(v) = self.limits.as_ref() {
            struct_ser.serialize_field("limits", v)?;
        }
        if let Some(v) = self.link_identity.as_ref() {
            struct_ser.serialize_field("linkIdentity", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ReadyResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "negotiation_id",
            "negotiationId",
            "profile_id",
            "profileId",
            "plane",
            "provides",
            "limits",
            "link_identity",
            "linkIdentity",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NegotiationId,
            ProfileId,
            Plane,
            Provides,
            Limits,
            LinkIdentity,
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
                            "negotiationId" | "negotiation_id" => Ok(GeneratedField::NegotiationId),
                            "profileId" | "profile_id" => Ok(GeneratedField::ProfileId),
                            "plane" => Ok(GeneratedField::Plane),
                            "provides" => Ok(GeneratedField::Provides),
                            "limits" => Ok(GeneratedField::Limits),
                            "linkIdentity" | "link_identity" => Ok(GeneratedField::LinkIdentity),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ReadyResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.ReadyResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ReadyResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut negotiation_id__ = None;
                let mut profile_id__ = None;
                let mut plane__ = None;
                let mut provides__ = None;
                let mut limits__ = None;
                let mut link_identity__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NegotiationId => {
                            if negotiation_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("negotiationId"));
                            }
                            negotiation_id__ = Some(map_.next_value()?);
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
                        GeneratedField::Limits => {
                            if limits__.is_some() {
                                return Err(serde::de::Error::duplicate_field("limits"));
                            }
                            limits__ = map_.next_value()?;
                        }
                        GeneratedField::LinkIdentity => {
                            if link_identity__.is_some() {
                                return Err(serde::de::Error::duplicate_field("linkIdentity"));
                            }
                            link_identity__ = map_.next_value()?;
                        }
                    }
                }
                Ok(ReadyResponse {
                    negotiation_id: negotiation_id__.unwrap_or_default(),
                    profile_id: profile_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    limits: limits__,
                    link_identity: link_identity__,
                })
            }
        }
        deserializer.deserialize_struct("conex.ReadyResponse", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.RejectedCapability", len)?;
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
                formatter.write_str("struct conex.RejectedCapability")
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
        deserializer.deserialize_struct("conex.RejectedCapability", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for RemoteTarget {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.endpoint_id.is_empty() {
            len += 1;
        }
        if !self.resource_id.is_empty() {
            len += 1;
        }
        if self.revision.is_some() {
            len += 1;
        }
        if !self.offset.is_empty() {
            len += 1;
        }
        if !self.length.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.RemoteTarget", len)?;
        if !self.endpoint_id.is_empty() {
            struct_ser.serialize_field("endpointId", &self.endpoint_id)?;
        }
        if !self.resource_id.is_empty() {
            struct_ser.serialize_field("resourceId", &self.resource_id)?;
        }
        if let Some(v) = self.revision.as_ref() {
            struct_ser.serialize_field("revision", v)?;
        }
        if !self.offset.is_empty() {
            struct_ser.serialize_field("offset", &self.offset)?;
        }
        if !self.length.is_empty() {
            struct_ser.serialize_field("length", &self.length)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RemoteTarget {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "endpoint_id",
            "endpointId",
            "resource_id",
            "resourceId",
            "revision",
            "offset",
            "length",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            EndpointId,
            ResourceId,
            Revision,
            Offset,
            Length,
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
                            "endpointId" | "endpoint_id" => Ok(GeneratedField::EndpointId),
                            "resourceId" | "resource_id" => Ok(GeneratedField::ResourceId),
                            "revision" => Ok(GeneratedField::Revision),
                            "offset" => Ok(GeneratedField::Offset),
                            "length" => Ok(GeneratedField::Length),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RemoteTarget;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.RemoteTarget")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RemoteTarget, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut endpoint_id__ = None;
                let mut resource_id__ = None;
                let mut revision__ = None;
                let mut offset__ = None;
                let mut length__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::EndpointId => {
                            if endpoint_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endpointId"));
                            }
                            endpoint_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ResourceId => {
                            if resource_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resourceId"));
                            }
                            resource_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = map_.next_value()?;
                        }
                        GeneratedField::Offset => {
                            if offset__.is_some() {
                                return Err(serde::de::Error::duplicate_field("offset"));
                            }
                            offset__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Length => {
                            if length__.is_some() {
                                return Err(serde::de::Error::duplicate_field("length"));
                            }
                            length__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(RemoteTarget {
                    endpoint_id: endpoint_id__.unwrap_or_default(),
                    resource_id: resource_id__.unwrap_or_default(),
                    revision: revision__,
                    offset: offset__.unwrap_or_default(),
                    length: length__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.RemoteTarget", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Request", len)?;
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
                formatter.write_str("struct conex.Request")
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
        deserializer.deserialize_struct("conex.Request", FIELDS, GeneratedVisitor)
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
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.RequestContext", len)?;
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
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
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
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ProviderEndpointId,
            Plane,
            BindingId,
            PrincipalId,
            TenantId,
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
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
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
                formatter.write_str("struct conex.RequestContext")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RequestContext, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut provider_endpoint_id__ = None;
                let mut plane__ = None;
                let mut binding_id__ = None;
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
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
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(RequestContext {
                    provider_endpoint_id: provider_endpoint_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    binding_id: binding_id__,
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.RequestContext", FIELDS, GeneratedVisitor)
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
        if self.kind != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.ResourceSummary", len)?;
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
            struct_ser.serialize_field("sizeBytes", v)?;
        }
        if let Some(v) = self.revision.as_ref() {
            struct_ser.serialize_field("revision", v)?;
        }
        if self.kind != 0 {
            let v = EntryKind::try_from(self.kind)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.kind)))?;
            struct_ser.serialize_field("kind", &v)?;
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
            "kind",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ResourceId,
            Title,
            Mime,
            SizeBytes,
            Revision,
            Kind,
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
                            "kind" => Ok(GeneratedField::Kind),
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
                formatter.write_str("struct conex.ResourceSummary")
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
                let mut kind__ = None;
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
                            size_bytes__ = map_.next_value()?;
                        }
                        GeneratedField::Revision => {
                            if revision__.is_some() {
                                return Err(serde::de::Error::duplicate_field("revision"));
                            }
                            revision__ = map_.next_value()?;
                        }
                        GeneratedField::Kind => {
                            if kind__.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind__ = Some(map_.next_value::<EntryKind>()? as i32);
                        }
                    }
                }
                Ok(ResourceSummary {
                    resource_id: resource_id__.unwrap_or_default(),
                    title: title__.unwrap_or_default(),
                    mime: mime__.unwrap_or_default(),
                    size_bytes: size_bytes__,
                    revision: revision__,
                    kind: kind__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.ResourceSummary", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SearchHit", len)?;
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
                formatter.write_str("struct conex.SearchHit")
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
        deserializer.deserialize_struct("conex.SearchHit", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionBinding {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if !self.provider_endpoint_id.is_empty() {
            len += 1;
        }
        if self.plane != 0 {
            len += 1;
        }
        if self.workspace_peer_id.is_some() {
            len += 1;
        }
        if self.human_peer_id.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionBinding", len)?;
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if !self.provider_endpoint_id.is_empty() {
            struct_ser.serialize_field("providerEndpointId", &self.provider_endpoint_id)?;
        }
        if self.plane != 0 {
            let v = Plane::try_from(self.plane)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.plane)))?;
            struct_ser.serialize_field("plane", &v)?;
        }
        if let Some(v) = self.workspace_peer_id.as_ref() {
            struct_ser.serialize_field("workspacePeerId", v)?;
        }
        if let Some(v) = self.human_peer_id.as_ref() {
            struct_ser.serialize_field("humanPeerId", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionBinding {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
            "provider_endpoint_id",
            "providerEndpointId",
            "plane",
            "workspace_peer_id",
            "workspacePeerId",
            "human_peer_id",
            "humanPeerId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            PrincipalId,
            TenantId,
            ProviderEndpointId,
            Plane,
            WorkspacePeerId,
            HumanPeerId,
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
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "providerEndpointId" | "provider_endpoint_id" => Ok(GeneratedField::ProviderEndpointId),
                            "plane" => Ok(GeneratedField::Plane),
                            "workspacePeerId" | "workspace_peer_id" => Ok(GeneratedField::WorkspacePeerId),
                            "humanPeerId" | "human_peer_id" => Ok(GeneratedField::HumanPeerId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionBinding;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionBinding")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionBinding, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
                let mut provider_endpoint_id__ = None;
                let mut plane__ = None;
                let mut workspace_peer_id__ = None;
                let mut human_peer_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
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
                        GeneratedField::WorkspacePeerId => {
                            if workspace_peer_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("workspacePeerId"));
                            }
                            workspace_peer_id__ = map_.next_value()?;
                        }
                        GeneratedField::HumanPeerId => {
                            if human_peer_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("humanPeerId"));
                            }
                            human_peer_id__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SessionBinding {
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    provider_endpoint_id: provider_endpoint_id__.unwrap_or_default(),
                    plane: plane__.unwrap_or_default(),
                    workspace_peer_id: workspace_peer_id__,
                    human_peer_id: human_peer_id__,
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionBinding", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionCloseRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.expected_epoch != 0 {
            len += 1;
        }
        if self.reason.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionCloseRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.expected_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expectedEpoch", ToString::to_string(&self.expected_epoch).as_str())?;
        }
        if let Some(v) = self.reason.as_ref() {
            struct_ser.serialize_field("reason", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionCloseRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "expected_epoch",
            "expectedEpoch",
            "reason",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            ExpectedEpoch,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "expectedEpoch" | "expected_epoch" => Ok(GeneratedField::ExpectedEpoch),
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
            type Value = SessionCloseRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionCloseRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionCloseRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut expected_epoch__ = None;
                let mut reason__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpectedEpoch => {
                            if expected_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expectedEpoch"));
                            }
                            expected_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SessionCloseRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    expected_epoch: expected_epoch__.unwrap_or_default(),
                    reason: reason__,
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionCloseRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionCloseResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if self.final_epoch != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionCloseResponse", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if self.final_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("finalEpoch", ToString::to_string(&self.final_epoch).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionCloseResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "final_epoch",
            "finalEpoch",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            FinalEpoch,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "finalEpoch" | "final_epoch" => Ok(GeneratedField::FinalEpoch),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionCloseResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionCloseResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionCloseResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut final_epoch__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::FinalEpoch => {
                            if final_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("finalEpoch"));
                            }
                            final_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(SessionCloseResponse {
                    session_id: session_id__.unwrap_or_default(),
                    final_epoch: final_epoch__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionCloseResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionOpenRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.requested.is_some() {
            len += 1;
        }
        if !self.recovery.is_empty() {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.requires.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionOpenRequest", len)?;
        if let Some(v) = self.requested.as_ref() {
            struct_ser.serialize_field("requested", v)?;
        }
        if !self.recovery.is_empty() {
            struct_ser.serialize_field("recovery", &self.recovery)?;
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
impl<'de> serde::Deserialize<'de> for SessionOpenRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "requested",
            "recovery",
            "provides",
            "requires",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Requested,
            Recovery,
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
                            "requested" => Ok(GeneratedField::Requested),
                            "recovery" => Ok(GeneratedField::Recovery),
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
            type Value = SessionOpenRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionOpenRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionOpenRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut requested__ = None;
                let mut recovery__ = None;
                let mut provides__ = None;
                let mut requires__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Requested => {
                            if requested__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requested"));
                            }
                            requested__ = map_.next_value()?;
                        }
                        GeneratedField::Recovery => {
                            if recovery__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recovery"));
                            }
                            recovery__ = Some(map_.next_value()?);
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
                Ok(SessionOpenRequest {
                    requested: requested__,
                    recovery: recovery__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    requires: requires__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionOpenRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionOpenResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if self.binding.is_some() {
            len += 1;
        }
        if !self.recovery.is_empty() {
            len += 1;
        }
        if self.attachment_epoch != 0 {
            len += 1;
        }
        if self.lease_ms != 0 {
            len += 1;
        }
        if !self.provides.is_empty() {
            len += 1;
        }
        if !self.rejected_capabilities.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionOpenResponse", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if let Some(v) = self.binding.as_ref() {
            struct_ser.serialize_field("binding", v)?;
        }
        if !self.recovery.is_empty() {
            struct_ser.serialize_field("recovery", &self.recovery)?;
        }
        if self.attachment_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("attachmentEpoch", ToString::to_string(&self.attachment_epoch).as_str())?;
        }
        if self.lease_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("leaseMs", ToString::to_string(&self.lease_ms).as_str())?;
        }
        if !self.provides.is_empty() {
            struct_ser.serialize_field("provides", &self.provides)?;
        }
        if !self.rejected_capabilities.is_empty() {
            struct_ser.serialize_field("rejectedCapabilities", &self.rejected_capabilities)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionOpenResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "binding",
            "recovery",
            "attachment_epoch",
            "attachmentEpoch",
            "lease_ms",
            "leaseMs",
            "provides",
            "rejected_capabilities",
            "rejectedCapabilities",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            Binding,
            Recovery,
            AttachmentEpoch,
            LeaseMs,
            Provides,
            RejectedCapabilities,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "binding" => Ok(GeneratedField::Binding),
                            "recovery" => Ok(GeneratedField::Recovery),
                            "attachmentEpoch" | "attachment_epoch" => Ok(GeneratedField::AttachmentEpoch),
                            "leaseMs" | "lease_ms" => Ok(GeneratedField::LeaseMs),
                            "provides" => Ok(GeneratedField::Provides),
                            "rejectedCapabilities" | "rejected_capabilities" => Ok(GeneratedField::RejectedCapabilities),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionOpenResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionOpenResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionOpenResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut binding__ = None;
                let mut recovery__ = None;
                let mut attachment_epoch__ = None;
                let mut lease_ms__ = None;
                let mut provides__ = None;
                let mut rejected_capabilities__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Binding => {
                            if binding__.is_some() {
                                return Err(serde::de::Error::duplicate_field("binding"));
                            }
                            binding__ = map_.next_value()?;
                        }
                        GeneratedField::Recovery => {
                            if recovery__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recovery"));
                            }
                            recovery__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentEpoch => {
                            if attachment_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentEpoch"));
                            }
                            attachment_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LeaseMs => {
                            if lease_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leaseMs"));
                            }
                            lease_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
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
                    }
                }
                Ok(SessionOpenResponse {
                    session_id: session_id__.unwrap_or_default(),
                    binding: binding__,
                    recovery: recovery__.unwrap_or_default(),
                    attachment_epoch: attachment_epoch__.unwrap_or_default(),
                    lease_ms: lease_ms__.unwrap_or_default(),
                    provides: provides__.unwrap_or_default(),
                    rejected_capabilities: rejected_capabilities__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionOpenResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionRenewRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.expected_epoch != 0 {
            len += 1;
        }
        if self.extend_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionRenewRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.expected_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expectedEpoch", ToString::to_string(&self.expected_epoch).as_str())?;
        }
        if self.extend_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("extendMs", ToString::to_string(&self.extend_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionRenewRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "expected_epoch",
            "expectedEpoch",
            "extend_ms",
            "extendMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            ExpectedEpoch,
            ExtendMs,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "expectedEpoch" | "expected_epoch" => Ok(GeneratedField::ExpectedEpoch),
                            "extendMs" | "extend_ms" => Ok(GeneratedField::ExtendMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionRenewRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionRenewRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionRenewRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut expected_epoch__ = None;
                let mut extend_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpectedEpoch => {
                            if expected_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expectedEpoch"));
                            }
                            expected_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ExtendMs => {
                            if extend_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("extendMs"));
                            }
                            extend_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(SessionRenewRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    expected_epoch: expected_epoch__.unwrap_or_default(),
                    extend_ms: extend_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionRenewRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionRenewResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.new_epoch != 0 {
            len += 1;
        }
        if self.lease_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionRenewResponse", len)?;
        if self.new_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("newEpoch", ToString::to_string(&self.new_epoch).as_str())?;
        }
        if self.lease_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("leaseMs", ToString::to_string(&self.lease_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionRenewResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "new_epoch",
            "newEpoch",
            "lease_ms",
            "leaseMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NewEpoch,
            LeaseMs,
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
                            "newEpoch" | "new_epoch" => Ok(GeneratedField::NewEpoch),
                            "leaseMs" | "lease_ms" => Ok(GeneratedField::LeaseMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionRenewResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionRenewResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionRenewResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut new_epoch__ = None;
                let mut lease_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NewEpoch => {
                            if new_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newEpoch"));
                            }
                            new_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LeaseMs => {
                            if lease_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leaseMs"));
                            }
                            lease_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(SessionRenewResponse {
                    new_epoch: new_epoch__.unwrap_or_default(),
                    lease_ms: lease_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionRenewResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionResumeRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.expected_epoch != 0 {
            len += 1;
        }
        if !self.streams.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionResumeRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.expected_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expectedEpoch", ToString::to_string(&self.expected_epoch).as_str())?;
        }
        if !self.streams.is_empty() {
            struct_ser.serialize_field("streams", &self.streams)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionResumeRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "expected_epoch",
            "expectedEpoch",
            "streams",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            ExpectedEpoch,
            Streams,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "expectedEpoch" | "expected_epoch" => Ok(GeneratedField::ExpectedEpoch),
                            "streams" => Ok(GeneratedField::Streams),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionResumeRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionResumeRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionResumeRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut expected_epoch__ = None;
                let mut streams__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ExpectedEpoch => {
                            if expected_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expectedEpoch"));
                            }
                            expected_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Streams => {
                            if streams__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streams"));
                            }
                            streams__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(SessionResumeRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    expected_epoch: expected_epoch__.unwrap_or_default(),
                    streams: streams__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionResumeRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SessionResumeResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.new_epoch != 0 {
            len += 1;
        }
        if !self.streams_reset.is_empty() {
            len += 1;
        }
        if self.restored_window_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SessionResumeResponse", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.new_epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("newEpoch", ToString::to_string(&self.new_epoch).as_str())?;
        }
        if !self.streams_reset.is_empty() {
            struct_ser.serialize_field("streamsReset", &self.streams_reset)?;
        }
        if self.restored_window_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("restoredWindowBytes", ToString::to_string(&self.restored_window_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SessionResumeResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "new_epoch",
            "newEpoch",
            "streams_reset",
            "streamsReset",
            "restored_window_bytes",
            "restoredWindowBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            NewEpoch,
            StreamsReset,
            RestoredWindowBytes,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "newEpoch" | "new_epoch" => Ok(GeneratedField::NewEpoch),
                            "streamsReset" | "streams_reset" => Ok(GeneratedField::StreamsReset),
                            "restoredWindowBytes" | "restored_window_bytes" => Ok(GeneratedField::RestoredWindowBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SessionResumeResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SessionResumeResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SessionResumeResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut new_epoch__ = None;
                let mut streams_reset__ = None;
                let mut restored_window_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NewEpoch => {
                            if new_epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newEpoch"));
                            }
                            new_epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StreamsReset => {
                            if streams_reset__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamsReset"));
                            }
                            streams_reset__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RestoredWindowBytes => {
                            if restored_window_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("restoredWindowBytes"));
                            }
                            restored_window_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(SessionResumeResponse {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    new_epoch: new_epoch__.unwrap_or_default(),
                    streams_reset: streams_reset__.unwrap_or_default(),
                    restored_window_bytes: restored_window_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SessionResumeResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for SlowConsumerSignal {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if !self.reason.is_empty() {
            len += 1;
        }
        if self.since_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SlowConsumerSignal", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if !self.reason.is_empty() {
            struct_ser.serialize_field("reason", &self.reason)?;
        }
        if self.since_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("sinceMs", ToString::to_string(&self.since_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for SlowConsumerSignal {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "reason",
            "since_ms",
            "sinceMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            Reason,
            SinceMs,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "reason" => Ok(GeneratedField::Reason),
                            "sinceMs" | "since_ms" => Ok(GeneratedField::SinceMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = SlowConsumerSignal;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.SlowConsumerSignal")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SlowConsumerSignal, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut reason__ = None;
                let mut since_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = Some(map_.next_value()?);
                        }
                        GeneratedField::SinceMs => {
                            if since_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sinceMs"));
                            }
                            since_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(SlowConsumerSignal {
                    stream_id: stream_id__.unwrap_or_default(),
                    reason: reason__.unwrap_or_default(),
                    since_ms: since_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.SlowConsumerSignal", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SourceListRequest", len)?;
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
                formatter.write_str("struct conex.SourceListRequest")
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
        deserializer.deserialize_struct("conex.SourceListRequest", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SourceListResponse", len)?;
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
                formatter.write_str("struct conex.SourceListResponse")
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
        deserializer.deserialize_struct("conex.SourceListResponse", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SourceReadRequest", len)?;
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
                formatter.write_str("struct conex.SourceReadRequest")
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
        deserializer.deserialize_struct("conex.SourceReadRequest", FIELDS, GeneratedVisitor)
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
        if self.text.is_some() {
            len += 1;
        }
        if self.cid.is_some() {
            len += 1;
        }
        if self.content.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.SourceReadResponse", len)?;
        if let Some(v) = self.resource.as_ref() {
            struct_ser.serialize_field("resource", v)?;
        }
        if let Some(v) = self.text.as_ref() {
            struct_ser.serialize_field("text", v)?;
        }
        if let Some(v) = self.cid.as_ref() {
            struct_ser.serialize_field("cid", v)?;
        }
        if let Some(v) = self.content.as_ref() {
            struct_ser.serialize_field("content", v)?;
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
            "content",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Resource,
            Text,
            Cid,
            Content,
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
                            "content" => Ok(GeneratedField::Content),
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
                formatter.write_str("struct conex.SourceReadResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<SourceReadResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut resource__ = None;
                let mut text__ = None;
                let mut cid__ = None;
                let mut content__ = None;
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
                            text__ = map_.next_value()?;
                        }
                        GeneratedField::Cid => {
                            if cid__.is_some() {
                                return Err(serde::de::Error::duplicate_field("cid"));
                            }
                            cid__ = map_.next_value()?;
                        }
                        GeneratedField::Content => {
                            if content__.is_some() {
                                return Err(serde::de::Error::duplicate_field("content"));
                            }
                            content__ = map_.next_value()?;
                        }
                    }
                }
                Ok(SourceReadResponse {
                    resource: resource__,
                    text: text__,
                    cid: cid__,
                    content: content__,
                })
            }
        }
        deserializer.deserialize_struct("conex.SourceReadResponse", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SourceSearchRequest", len)?;
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
                formatter.write_str("struct conex.SourceSearchRequest")
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
        deserializer.deserialize_struct("conex.SourceSearchRequest", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.SourceSearchResponse", len)?;
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
                formatter.write_str("struct conex.SourceSearchResponse")
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
        deserializer.deserialize_struct("conex.SourceSearchResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamAckRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.epoch != 0 {
            len += 1;
        }
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.last_received_seq != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamAckRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("epoch", ToString::to_string(&self.epoch).as_str())?;
        }
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.last_received_seq != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastReceivedSeq", ToString::to_string(&self.last_received_seq).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamAckRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "epoch",
            "stream_id",
            "streamId",
            "last_received_seq",
            "lastReceivedSeq",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            Epoch,
            StreamId,
            LastReceivedSeq,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "epoch" => Ok(GeneratedField::Epoch),
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "lastReceivedSeq" | "last_received_seq" => Ok(GeneratedField::LastReceivedSeq),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamAckRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamAckRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamAckRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut epoch__ = None;
                let mut stream_id__ = None;
                let mut last_received_seq__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Epoch => {
                            if epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("epoch"));
                            }
                            epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LastReceivedSeq => {
                            if last_received_seq__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastReceivedSeq"));
                            }
                            last_received_seq__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamAckRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    epoch: epoch__.unwrap_or_default(),
                    stream_id: stream_id__.unwrap_or_default(),
                    last_received_seq: last_received_seq__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamAckRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamAckResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.available_window_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamAckResponse", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.available_window_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("availableWindowBytes", ToString::to_string(&self.available_window_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamAckResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "available_window_bytes",
            "availableWindowBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            AvailableWindowBytes,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "availableWindowBytes" | "available_window_bytes" => Ok(GeneratedField::AvailableWindowBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamAckResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamAckResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamAckResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut available_window_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AvailableWindowBytes => {
                            if available_window_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("availableWindowBytes"));
                            }
                            available_window_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamAckResponse {
                    stream_id: stream_id__.unwrap_or_default(),
                    available_window_bytes: available_window_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamAckResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamCursor {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.last_received_seq != 0 {
            len += 1;
        }
        if self.consumed_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamCursor", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.last_received_seq != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastReceivedSeq", ToString::to_string(&self.last_received_seq).as_str())?;
        }
        if self.consumed_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("consumedBytes", ToString::to_string(&self.consumed_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamCursor {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "last_received_seq",
            "lastReceivedSeq",
            "consumed_bytes",
            "consumedBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            LastReceivedSeq,
            ConsumedBytes,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "lastReceivedSeq" | "last_received_seq" => Ok(GeneratedField::LastReceivedSeq),
                            "consumedBytes" | "consumed_bytes" => Ok(GeneratedField::ConsumedBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamCursor;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamCursor")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamCursor, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut last_received_seq__ = None;
                let mut consumed_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::LastReceivedSeq => {
                            if last_received_seq__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastReceivedSeq"));
                            }
                            last_received_seq__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ConsumedBytes => {
                            if consumed_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consumedBytes"));
                            }
                            consumed_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamCursor {
                    stream_id: stream_id__.unwrap_or_default(),
                    last_received_seq: last_received_seq__.unwrap_or_default(),
                    consumed_bytes: consumed_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamCursor", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamFlowRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.epoch != 0 {
            len += 1;
        }
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.consumed_bytes != 0 {
            len += 1;
        }
        if self.requested_window_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamFlowRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("epoch", ToString::to_string(&self.epoch).as_str())?;
        }
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.consumed_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("consumedBytes", ToString::to_string(&self.consumed_bytes).as_str())?;
        }
        if self.requested_window_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("requestedWindowBytes", ToString::to_string(&self.requested_window_bytes).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamFlowRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "epoch",
            "stream_id",
            "streamId",
            "consumed_bytes",
            "consumedBytes",
            "requested_window_bytes",
            "requestedWindowBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            Epoch,
            StreamId,
            ConsumedBytes,
            RequestedWindowBytes,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "epoch" => Ok(GeneratedField::Epoch),
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "consumedBytes" | "consumed_bytes" => Ok(GeneratedField::ConsumedBytes),
                            "requestedWindowBytes" | "requested_window_bytes" => Ok(GeneratedField::RequestedWindowBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamFlowRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamFlowRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamFlowRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut epoch__ = None;
                let mut stream_id__ = None;
                let mut consumed_bytes__ = None;
                let mut requested_window_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Epoch => {
                            if epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("epoch"));
                            }
                            epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ConsumedBytes => {
                            if consumed_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consumedBytes"));
                            }
                            consumed_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RequestedWindowBytes => {
                            if requested_window_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("requestedWindowBytes"));
                            }
                            requested_window_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamFlowRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    epoch: epoch__.unwrap_or_default(),
                    stream_id: stream_id__.unwrap_or_default(),
                    consumed_bytes: consumed_bytes__.unwrap_or_default(),
                    requested_window_bytes: requested_window_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamFlowRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamFlowResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if self.new_window_bytes != 0 {
            len += 1;
        }
        if self.zero_window {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamFlowResponse", len)?;
        if self.new_window_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("newWindowBytes", ToString::to_string(&self.new_window_bytes).as_str())?;
        }
        if self.zero_window {
            struct_ser.serialize_field("zeroWindow", &self.zero_window)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamFlowResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "new_window_bytes",
            "newWindowBytes",
            "zero_window",
            "zeroWindow",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NewWindowBytes,
            ZeroWindow,
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
                            "newWindowBytes" | "new_window_bytes" => Ok(GeneratedField::NewWindowBytes),
                            "zeroWindow" | "zero_window" => Ok(GeneratedField::ZeroWindow),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamFlowResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamFlowResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamFlowResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut new_window_bytes__ = None;
                let mut zero_window__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NewWindowBytes => {
                            if new_window_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newWindowBytes"));
                            }
                            new_window_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ZeroWindow => {
                            if zero_window__.is_some() {
                                return Err(serde::de::Error::duplicate_field("zeroWindow"));
                            }
                            zero_window__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(StreamFlowResponse {
                    new_window_bytes: new_window_bytes__.unwrap_or_default(),
                    zero_window: zero_window__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamFlowResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamFrame {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.epoch != 0 {
            len += 1;
        }
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.seq != 0 {
            len += 1;
        }
        if !self.message.is_empty() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamFrame", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("epoch", ToString::to_string(&self.epoch).as_str())?;
        }
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.seq != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("seq", ToString::to_string(&self.seq).as_str())?;
        }
        if !self.message.is_empty() {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("message", pbjson::private::base64::encode(&self.message).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamFrame {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "epoch",
            "stream_id",
            "streamId",
            "seq",
            "message",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            Epoch,
            StreamId,
            Seq,
            Message,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "epoch" => Ok(GeneratedField::Epoch),
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "seq" => Ok(GeneratedField::Seq),
                            "message" => Ok(GeneratedField::Message),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamFrame;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamFrame")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamFrame, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut epoch__ = None;
                let mut stream_id__ = None;
                let mut seq__ = None;
                let mut message__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Epoch => {
                            if epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("epoch"));
                            }
                            epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Seq => {
                            if seq__.is_some() {
                                return Err(serde::de::Error::duplicate_field("seq"));
                            }
                            seq__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Message => {
                            if message__.is_some() {
                                return Err(serde::de::Error::duplicate_field("message"));
                            }
                            message__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamFrame {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    epoch: epoch__.unwrap_or_default(),
                    stream_id: stream_id__.unwrap_or_default(),
                    seq: seq__.unwrap_or_default(),
                    message: message__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamFrame", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamReset {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if !self.reason.is_empty() {
            len += 1;
        }
        if self.resume_handle.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamReset", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if !self.reason.is_empty() {
            struct_ser.serialize_field("reason", &self.reason)?;
        }
        if let Some(v) = self.resume_handle.as_ref() {
            struct_ser.serialize_field("resumeHandle", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamReset {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "reason",
            "resume_handle",
            "resumeHandle",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            Reason,
            ResumeHandle,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "reason" => Ok(GeneratedField::Reason),
                            "resumeHandle" | "resume_handle" => Ok(GeneratedField::ResumeHandle),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamReset;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamReset")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamReset, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut reason__ = None;
                let mut resume_handle__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ResumeHandle => {
                            if resume_handle__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resumeHandle"));
                            }
                            resume_handle__ = map_.next_value()?;
                        }
                    }
                }
                Ok(StreamReset {
                    stream_id: stream_id__.unwrap_or_default(),
                    reason: reason__.unwrap_or_default(),
                    resume_handle: resume_handle__,
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamReset", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamResetRequest {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.session_id.is_empty() {
            len += 1;
        }
        if !self.attachment_id.is_empty() {
            len += 1;
        }
        if self.epoch != 0 {
            len += 1;
        }
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.after_seq != 0 {
            len += 1;
        }
        if !self.reason.is_empty() {
            len += 1;
        }
        if self.resume_handle.is_some() {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamResetRequest", len)?;
        if !self.session_id.is_empty() {
            struct_ser.serialize_field("sessionId", &self.session_id)?;
        }
        if !self.attachment_id.is_empty() {
            struct_ser.serialize_field("attachmentId", &self.attachment_id)?;
        }
        if self.epoch != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("epoch", ToString::to_string(&self.epoch).as_str())?;
        }
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.after_seq != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("afterSeq", ToString::to_string(&self.after_seq).as_str())?;
        }
        if !self.reason.is_empty() {
            struct_ser.serialize_field("reason", &self.reason)?;
        }
        if let Some(v) = self.resume_handle.as_ref() {
            struct_ser.serialize_field("resumeHandle", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamResetRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "session_id",
            "sessionId",
            "attachment_id",
            "attachmentId",
            "epoch",
            "stream_id",
            "streamId",
            "after_seq",
            "afterSeq",
            "reason",
            "resume_handle",
            "resumeHandle",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            SessionId,
            AttachmentId,
            Epoch,
            StreamId,
            AfterSeq,
            Reason,
            ResumeHandle,
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
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "attachmentId" | "attachment_id" => Ok(GeneratedField::AttachmentId),
                            "epoch" => Ok(GeneratedField::Epoch),
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "afterSeq" | "after_seq" => Ok(GeneratedField::AfterSeq),
                            "reason" => Ok(GeneratedField::Reason),
                            "resumeHandle" | "resume_handle" => Ok(GeneratedField::ResumeHandle),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamResetRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamResetRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamResetRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut session_id__ = None;
                let mut attachment_id__ = None;
                let mut epoch__ = None;
                let mut stream_id__ = None;
                let mut after_seq__ = None;
                let mut reason__ = None;
                let mut resume_handle__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AttachmentId => {
                            if attachment_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("attachmentId"));
                            }
                            attachment_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Epoch => {
                            if epoch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("epoch"));
                            }
                            epoch__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AfterSeq => {
                            if after_seq__.is_some() {
                                return Err(serde::de::Error::duplicate_field("afterSeq"));
                            }
                            after_seq__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Reason => {
                            if reason__.is_some() {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ResumeHandle => {
                            if resume_handle__.is_some() {
                                return Err(serde::de::Error::duplicate_field("resumeHandle"));
                            }
                            resume_handle__ = map_.next_value()?;
                        }
                    }
                }
                Ok(StreamResetRequest {
                    session_id: session_id__.unwrap_or_default(),
                    attachment_id: attachment_id__.unwrap_or_default(),
                    epoch: epoch__.unwrap_or_default(),
                    stream_id: stream_id__.unwrap_or_default(),
                    after_seq: after_seq__.unwrap_or_default(),
                    reason: reason__.unwrap_or_default(),
                    resume_handle: resume_handle__,
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamResetRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamResetResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if self.accepted_after_seq != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamResetResponse", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if self.accepted_after_seq != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("acceptedAfterSeq", ToString::to_string(&self.accepted_after_seq).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamResetResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "accepted_after_seq",
            "acceptedAfterSeq",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            AcceptedAfterSeq,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "acceptedAfterSeq" | "accepted_after_seq" => Ok(GeneratedField::AcceptedAfterSeq),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamResetResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamResetResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamResetResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut accepted_after_seq__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::AcceptedAfterSeq => {
                            if accepted_after_seq__.is_some() {
                                return Err(serde::de::Error::duplicate_field("acceptedAfterSeq"));
                            }
                            accepted_after_seq__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamResetResponse {
                    stream_id: stream_id__.unwrap_or_default(),
                    accepted_after_seq: accepted_after_seq__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamResetResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for StreamSpec {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.stream_id.is_empty() {
            len += 1;
        }
        if !self.kind.is_empty() {
            len += 1;
        }
        if self.window_bytes != 0 {
            len += 1;
        }
        if self.max_frame_bytes != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.StreamSpec", len)?;
        if !self.stream_id.is_empty() {
            struct_ser.serialize_field("streamId", &self.stream_id)?;
        }
        if !self.kind.is_empty() {
            struct_ser.serialize_field("kind", &self.kind)?;
        }
        if self.window_bytes != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("windowBytes", ToString::to_string(&self.window_bytes).as_str())?;
        }
        if self.max_frame_bytes != 0 {
            struct_ser.serialize_field("maxFrameBytes", &self.max_frame_bytes)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for StreamSpec {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "stream_id",
            "streamId",
            "kind",
            "window_bytes",
            "windowBytes",
            "max_frame_bytes",
            "maxFrameBytes",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            StreamId,
            Kind,
            WindowBytes,
            MaxFrameBytes,
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
                            "streamId" | "stream_id" => Ok(GeneratedField::StreamId),
                            "kind" => Ok(GeneratedField::Kind),
                            "windowBytes" | "window_bytes" => Ok(GeneratedField::WindowBytes),
                            "maxFrameBytes" | "max_frame_bytes" => Ok(GeneratedField::MaxFrameBytes),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = StreamSpec;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.StreamSpec")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<StreamSpec, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut stream_id__ = None;
                let mut kind__ = None;
                let mut window_bytes__ = None;
                let mut max_frame_bytes__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::StreamId => {
                            if stream_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("streamId"));
                            }
                            stream_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Kind => {
                            if kind__.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind__ = Some(map_.next_value()?);
                        }
                        GeneratedField::WindowBytes => {
                            if window_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("windowBytes"));
                            }
                            window_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MaxFrameBytes => {
                            if max_frame_bytes__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxFrameBytes"));
                            }
                            max_frame_bytes__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(StreamSpec {
                    stream_id: stream_id__.unwrap_or_default(),
                    kind: kind__.unwrap_or_default(),
                    window_bytes: window_bytes__.unwrap_or_default(),
                    max_frame_bytes: max_frame_bytes__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.StreamSpec", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("conex.Success", len)?;
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
                formatter.write_str("struct conex.Success")
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
        deserializer.deserialize_struct("conex.Success", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for UiLinkSummary {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.link_id.is_empty() {
            len += 1;
        }
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if self.connected_at_ms != 0 {
            len += 1;
        }
        if self.last_seen_at_ms != 0 {
            len += 1;
        }
        if self.tickets_issued != 0 {
            len += 1;
        }
        if self.calls_total != 0 {
            len += 1;
        }
        if self.calls_in_flight != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.UiLinkSummary", len)?;
        if !self.link_id.is_empty() {
            struct_ser.serialize_field("linkId", &self.link_id)?;
        }
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if self.connected_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("connectedAtMs", ToString::to_string(&self.connected_at_ms).as_str())?;
        }
        if self.last_seen_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("lastSeenAtMs", ToString::to_string(&self.last_seen_at_ms).as_str())?;
        }
        if self.tickets_issued != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("ticketsIssued", ToString::to_string(&self.tickets_issued).as_str())?;
        }
        if self.calls_total != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("callsTotal", ToString::to_string(&self.calls_total).as_str())?;
        }
        if self.calls_in_flight != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("callsInFlight", ToString::to_string(&self.calls_in_flight).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for UiLinkSummary {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "link_id",
            "linkId",
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
            "connected_at_ms",
            "connectedAtMs",
            "last_seen_at_ms",
            "lastSeenAtMs",
            "tickets_issued",
            "ticketsIssued",
            "calls_total",
            "callsTotal",
            "calls_in_flight",
            "callsInFlight",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            LinkId,
            PrincipalId,
            TenantId,
            ConnectedAtMs,
            LastSeenAtMs,
            TicketsIssued,
            CallsTotal,
            CallsInFlight,
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
                            "linkId" | "link_id" => Ok(GeneratedField::LinkId),
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "connectedAtMs" | "connected_at_ms" => Ok(GeneratedField::ConnectedAtMs),
                            "lastSeenAtMs" | "last_seen_at_ms" => Ok(GeneratedField::LastSeenAtMs),
                            "ticketsIssued" | "tickets_issued" => Ok(GeneratedField::TicketsIssued),
                            "callsTotal" | "calls_total" => Ok(GeneratedField::CallsTotal),
                            "callsInFlight" | "calls_in_flight" => Ok(GeneratedField::CallsInFlight),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = UiLinkSummary;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.UiLinkSummary")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<UiLinkSummary, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut link_id__ = None;
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
                let mut connected_at_ms__ = None;
                let mut last_seen_at_ms__ = None;
                let mut tickets_issued__ = None;
                let mut calls_total__ = None;
                let mut calls_in_flight__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::LinkId => {
                            if link_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("linkId"));
                            }
                            link_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ConnectedAtMs => {
                            if connected_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("connectedAtMs"));
                            }
                            connected_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastSeenAtMs => {
                            if last_seen_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastSeenAtMs"));
                            }
                            last_seen_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::TicketsIssued => {
                            if tickets_issued__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ticketsIssued"));
                            }
                            tickets_issued__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::CallsTotal => {
                            if calls_total__.is_some() {
                                return Err(serde::de::Error::duplicate_field("callsTotal"));
                            }
                            calls_total__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::CallsInFlight => {
                            if calls_in_flight__.is_some() {
                                return Err(serde::de::Error::duplicate_field("callsInFlight"));
                            }
                            calls_in_flight__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(UiLinkSummary {
                    link_id: link_id__.unwrap_or_default(),
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    connected_at_ms: connected_at_ms__.unwrap_or_default(),
                    last_seen_at_ms: last_seen_at_ms__.unwrap_or_default(),
                    tickets_issued: tickets_issued__.unwrap_or_default(),
                    calls_total: calls_total__.unwrap_or_default(),
                    calls_in_flight: calls_in_flight__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.UiLinkSummary", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for WebTicket {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if !self.ticket.is_empty() {
            len += 1;
        }
        if !self.principal_id.is_empty() {
            len += 1;
        }
        if !self.tenant_id.is_empty() {
            len += 1;
        }
        if !self.origin.is_empty() {
            len += 1;
        }
        if !self.target_host.is_empty() {
            len += 1;
        }
        if !self.peer_role.is_empty() {
            len += 1;
        }
        if !self.capability_caps.is_empty() {
            len += 1;
        }
        if self.session_id.is_some() {
            len += 1;
        }
        if self.issued_at_ms != 0 {
            len += 1;
        }
        if self.expires_at_ms != 0 {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("conex.WebTicket", len)?;
        if !self.ticket.is_empty() {
            struct_ser.serialize_field("ticket", &self.ticket)?;
        }
        if !self.principal_id.is_empty() {
            struct_ser.serialize_field("principalId", &self.principal_id)?;
        }
        if !self.tenant_id.is_empty() {
            struct_ser.serialize_field("tenantId", &self.tenant_id)?;
        }
        if !self.origin.is_empty() {
            struct_ser.serialize_field("origin", &self.origin)?;
        }
        if !self.target_host.is_empty() {
            struct_ser.serialize_field("targetHost", &self.target_host)?;
        }
        if !self.peer_role.is_empty() {
            struct_ser.serialize_field("peerRole", &self.peer_role)?;
        }
        if !self.capability_caps.is_empty() {
            struct_ser.serialize_field("capabilityCaps", &self.capability_caps)?;
        }
        if let Some(v) = self.session_id.as_ref() {
            struct_ser.serialize_field("sessionId", v)?;
        }
        if self.issued_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("issuedAtMs", ToString::to_string(&self.issued_at_ms).as_str())?;
        }
        if self.expires_at_ms != 0 {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("expiresAtMs", ToString::to_string(&self.expires_at_ms).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for WebTicket {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "ticket",
            "principal_id",
            "principalId",
            "tenant_id",
            "tenantId",
            "origin",
            "target_host",
            "targetHost",
            "peer_role",
            "peerRole",
            "capability_caps",
            "capabilityCaps",
            "session_id",
            "sessionId",
            "issued_at_ms",
            "issuedAtMs",
            "expires_at_ms",
            "expiresAtMs",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Ticket,
            PrincipalId,
            TenantId,
            Origin,
            TargetHost,
            PeerRole,
            CapabilityCaps,
            SessionId,
            IssuedAtMs,
            ExpiresAtMs,
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
                            "ticket" => Ok(GeneratedField::Ticket),
                            "principalId" | "principal_id" => Ok(GeneratedField::PrincipalId),
                            "tenantId" | "tenant_id" => Ok(GeneratedField::TenantId),
                            "origin" => Ok(GeneratedField::Origin),
                            "targetHost" | "target_host" => Ok(GeneratedField::TargetHost),
                            "peerRole" | "peer_role" => Ok(GeneratedField::PeerRole),
                            "capabilityCaps" | "capability_caps" => Ok(GeneratedField::CapabilityCaps),
                            "sessionId" | "session_id" => Ok(GeneratedField::SessionId),
                            "issuedAtMs" | "issued_at_ms" => Ok(GeneratedField::IssuedAtMs),
                            "expiresAtMs" | "expires_at_ms" => Ok(GeneratedField::ExpiresAtMs),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = WebTicket;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct conex.WebTicket")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<WebTicket, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut ticket__ = None;
                let mut principal_id__ = None;
                let mut tenant_id__ = None;
                let mut origin__ = None;
                let mut target_host__ = None;
                let mut peer_role__ = None;
                let mut capability_caps__ = None;
                let mut session_id__ = None;
                let mut issued_at_ms__ = None;
                let mut expires_at_ms__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Ticket => {
                            if ticket__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ticket"));
                            }
                            ticket__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PrincipalId => {
                            if principal_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("principalId"));
                            }
                            principal_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TenantId => {
                            if tenant_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tenantId"));
                            }
                            tenant_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Origin => {
                            if origin__.is_some() {
                                return Err(serde::de::Error::duplicate_field("origin"));
                            }
                            origin__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TargetHost => {
                            if target_host__.is_some() {
                                return Err(serde::de::Error::duplicate_field("targetHost"));
                            }
                            target_host__ = Some(map_.next_value()?);
                        }
                        GeneratedField::PeerRole => {
                            if peer_role__.is_some() {
                                return Err(serde::de::Error::duplicate_field("peerRole"));
                            }
                            peer_role__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CapabilityCaps => {
                            if capability_caps__.is_some() {
                                return Err(serde::de::Error::duplicate_field("capabilityCaps"));
                            }
                            capability_caps__ = Some(map_.next_value()?);
                        }
                        GeneratedField::SessionId => {
                            if session_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sessionId"));
                            }
                            session_id__ = map_.next_value()?;
                        }
                        GeneratedField::IssuedAtMs => {
                            if issued_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("issuedAtMs"));
                            }
                            issued_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ExpiresAtMs => {
                            if expires_at_ms__.is_some() {
                                return Err(serde::de::Error::duplicate_field("expiresAtMs"));
                            }
                            expires_at_ms__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(WebTicket {
                    ticket: ticket__.unwrap_or_default(),
                    principal_id: principal_id__.unwrap_or_default(),
                    tenant_id: tenant_id__.unwrap_or_default(),
                    origin: origin__.unwrap_or_default(),
                    target_host: target_host__.unwrap_or_default(),
                    peer_role: peer_role__.unwrap_or_default(),
                    capability_caps: capability_caps__.unwrap_or_default(),
                    session_id: session_id__,
                    issued_at_ms: issued_at_ms__.unwrap_or_default(),
                    expires_at_ms: expires_at_ms__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("conex.WebTicket", FIELDS, GeneratedVisitor)
    }
}
