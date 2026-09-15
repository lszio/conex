# P0 wire 契约（conex-jsonrpc2-http-v1）

状态：P0 冻结草案，对应设计 v6 §5.2 与 P0 计划 Task 02。
类型源：`schema/conex/v1/*.proto`；派生产物见 `schema/generated/`。
权威严格解码点是 `MethodContract.prepare`/`validate_output`；JSON Schema 只作文档与 TS 边界输入。

## 1. 信封

固定入口 `POST /rpc`，Content-Type `application/json`。四个变体：

```text
Request      { jsonrpc:"2.0", id:ULID, method, params }
Success      { jsonrpc:"2.0", id:ULID, result }
Failure      { jsonrpc:"2.0", id:ULID|null, error }
Notification { jsonrpc:"2.0", method, params }
```

- `id` 必须是 26 字符 Crockford base32 ULID 字符串（首字符 ≤ `7`）；通知省略 `id`；解析错误无法关联时用 `id:null`。
- 顶层只允许 `jsonrpc/id/method/params/result/error`；未知字段与重复字段一律拒绝（重复键不得先被 map 覆盖）。
- 不支持 JSON-RPC batch；收到数组直接拒绝，不执行其中任何一项。
- `method` 不能与 `result`/`error` 同时出现；`success`/`error` 不能同时出现。

## 2. params

```json
{
  "context": { "providerEndpointId": "...", "plane": "broker", "bindingId": "..." },
  "timeoutBudgetMs": 8000,
  "input": { }
}
```

- `context` 只用于选择目标；主体、租户与策略由接收侧派生，客户端提交的 principal/policyVersion 被忽略或拒绝。
- `plane` 在 P0 只接受 `broker`；`relay` 可识别但在 `validate_message` 阶段返回 `plane_mismatch`。
- `timeoutBudgetMs` 必须大于 0；跨跳沿用剩余预算。
- `input` 是通用容器，具体类型由方法注册引用生成的 typed message 并在 `prepare` 中严格解码。

## 3. 错误

```json
{ "code": -32002, "message": "...",
  "data": { "code": "forbidden", "diagnosticId": "...", "execution": "not_started", "retry": "never", "details": {} } }
```

- `error.code` 是冻结的数值（见下表）；`error.data.code` 是从同名枚举派生的小写语义名。
- `execution ∈ {not_started, completed, unknown}`；`retry ∈ {never, safe, with_operation_id}`。

| 语义码 | 数值 | 语义码 | 数值 |
|---|---|---|---|
| parse_error | -32700 | unauthorized | -32001 |
| bad_request | -32602 | forbidden | -32002 |
| unknown_method | -32601 | unknown_provider | -32003 |
| internal | -32603 | unsupported_capability | -32004 |
| unavailable | -32005 | timeout | -32006 |
| payload_too_large | -32007 | quota_exceeded | -32008 |
| plane_mismatch | -32009 | peer_untrusted | -32010 |
| cancelled | -32011 | outcome_unknown | -32012 |
| stale_revision | -32013 | conflict | -32014 |
| slow_consumer | -32015 | bad_blob | -32016 |
| session_lost | -32017 | resume_unavailable | -32018 |

`-32019..-32099` 预留。新增枚举值必须同步更新 `wire::error_code_name`（该 match 对 Ok 变体穷尽，遗漏会编译失败）。

## 4. HTTP 行为

- 认证失败 401；媒体类型错误 415；body 超限 413；其余已解码调用返回 200；有效通知返回 204。
- 不接受 batch；孤立 response 对象拒绝并审计。
- `conex/hello` 在有认证、无 binding 的请求中调用，返回 `bindingId` + `expiresInMs=60000` + `profileId/plane/provides/rejectedCapabilities/limits`。

## 5. 预留

`CallParams` tag 4/5 与 `Success/Failure/Notification` tag 3 为 `operationId`/`meta` 预留，P0 不使用也不得改号或复用（设计 §5.2）。

## 6. 验证

`cargo test -p conex-proto --test wire` 读取 `conformance/vectors/p0/wire.json`；
TS 端 `sdk/typescript/tests/wire.test.ts` 校验生成的 `ErrorCode` 枚举与同一张数值表一致。
