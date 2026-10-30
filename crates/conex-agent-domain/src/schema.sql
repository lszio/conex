-- Agent 领域初始 schema（docs/plans/2026-10-29-conex-mvp-p1.md §3）。
-- 所有外键目标在同库；JSON 字段为生成记录序列化，不是未验证用户对象。
-- 迁移由 store 代码以事务执行；旧库版本高于程序直接拒绝。
PRAGMA foreign_keys=ON;
CREATE TABLE store_identity(node_id TEXT PRIMARY KEY);
CREATE TABLE streams(
 id TEXT PRIMARY KEY, epoch TEXT NOT NULL,
 head INTEGER NOT NULL DEFAULT 0 CHECK(head>=0),
 first_available INTEGER NOT NULL DEFAULT 1 CHECK(first_available>=1),
 event_bytes INTEGER NOT NULL DEFAULT 0 CHECK(event_bytes>=0)
);
CREATE TABLE conversations(
 id TEXT PRIMARY KEY REFERENCES streams(id), endpoint_id TEXT NOT NULL,
 workspace_id TEXT NOT NULL, active_execution_id TEXT,
 revision INTEGER NOT NULL CHECK(revision>=1), projection BLOB NOT NULL
);
CREATE TABLE executions(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 parent_execution_id TEXT REFERENCES executions(id), generation INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN
 ('accepted','dispatching','running','unknown','succeeded','failed','cancelled')),
 revision INTEGER NOT NULL CHECK(revision>=1), projection BLOB NOT NULL
);
CREATE UNIQUE INDEX one_top_level_active ON executions(conversation_id)
 WHERE parent_execution_id IS NULL
 AND state IN ('accepted','dispatching','running','unknown');
CREATE TABLE interactions(
 id TEXT PRIMARY KEY, execution_id TEXT NOT NULL REFERENCES executions(id),
 state TEXT NOT NULL CHECK(state IN
 ('pending','responding','resolved','expired','invalidated','unknown')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 expires_at_ms INTEGER NOT NULL, decision_digest TEXT, responder_id TEXT,
 projection BLOB NOT NULL
);
CREATE TABLE messages(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 execution_id TEXT NOT NULL REFERENCES executions(id), ordinal INTEGER NOT NULL,
 backend_key TEXT, revision INTEGER NOT NULL CHECK(revision>=1),
 state TEXT NOT NULL CHECK(state IN ('streaming','complete','incomplete')), projection BLOB NOT NULL,
 UNIQUE(conversation_id,ordinal), UNIQUE(conversation_id,backend_key)
);
CREATE TABLE backend_mappings(
 adapter TEXT NOT NULL, backend_instance_key TEXT NOT NULL,
 backend_conversation_id TEXT NOT NULL, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 PRIMARY KEY(adapter,backend_instance_key,backend_conversation_id)
);
CREATE TABLE entities(
 id TEXT PRIMARY KEY, kind TEXT NOT NULL, revision INTEGER NOT NULL, projection BLOB NOT NULL
);
CREATE TABLE dedup(
 principal_id TEXT NOT NULL, endpoint_id TEXT NOT NULL, operation TEXT NOT NULL,
 key TEXT NOT NULL, digest TEXT NOT NULL, object_id TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
 PRIMARY KEY(principal_id,endpoint_id,operation,key)
);
CREATE TABLE events(
 stream_id TEXT NOT NULL REFERENCES streams(id), position INTEGER NOT NULL CHECK(position>0),
 recorded_at_ms INTEGER NOT NULL, encoded BLOB NOT NULL,
 encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes>0),
 PRIMARY KEY(stream_id,position)
);
CREATE INDEX events_age ON events(recorded_at_ms,stream_id,position);

-- v2（P2.2）：持久身份、模块代际与自有资源所有权（docs/plans/
-- 2026-10-29-conex-mvp-p2.md §3）。旧 v1 库由 store.rs 备份一致快照后
-- 原子迁移补充本段，不覆盖 P1 表。迁移 DDL 必须与 store.rs 的
-- MIGRATE_V1_TO_V2 保持一致。
CREATE TABLE node_metadata(
 node_id TEXT PRIMARY KEY,
 runtime_epoch TEXT NOT NULL
);
CREATE TABLE module_generations(
 module_key TEXT PRIMARY KEY,
 generation INTEGER NOT NULL CHECK(generation>0)
);
CREATE TABLE owned_resources(
 id TEXT PRIMARY KEY, module_key TEXT NOT NULL, generation INTEGER NOT NULL,
 owner_epoch TEXT NOT NULL, control_kind TEXT NOT NULL, control_identity BLOB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('created','active','unknown','released'))
);
PRAGMA user_version=2;
