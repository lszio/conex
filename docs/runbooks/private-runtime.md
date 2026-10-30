# MVP-P2 自治节点运行手册

## 已交付边界

P2 在现有 `conex-agent` 中增加 `--node-config` 模式；旧 `--config` source-provider 模式保留。应用包只引用节点静态批准的实现与配置，不执行包提供的命令，不下载代码。当前实际实现为 `controlled-counter` 和 `external-counter`，均提供 `conex.counter` 的 `counter/read`、`counter/add`；**未安装 Agent/OMP/Codex handler**，未知能力明确拒绝，不返回假成功。

本地入口仅为受限 Unix socket，目录0700、socket0600，并检查 OS peer UID。配置的 operator principal 由节点构造可信 Caller；请求不能自报 principal/tenant/registration。P5 的 JWT、Web/Emacs 直连及公网认证尚未交付。`direct-only` 当前只可经本地 API 消费，不等于已经提供浏览器直连。

## 启动样例

从仓库根运行：

```sh
cargo run -p conex-agent -- --node-config deployment/agent-a.toml.example
```

样例使用 `/tmp/conex-node-a` 和 `examples/mvp/node/application.toml`。安装时更换 node/operator ULID、node_ref、受限持久目录及 socket；相同目录只允许一个运行中节点。样例 counter 为 `direct-only`，中心离线或凭据环境变量未设置也能启动本地服务；没有可转发导出时不连接中心。正式安装不要把持久数据放在 `/tmp`。

`publication.token_backend` 只支持管理员批准的 `env:NAME` / `file:/absolute/path` 凭据引用。文件必须是受限常规文件；不把值写进包、配置、URL或进程参数。启用 relay 时还需实际中心的批准身份与精确 exposure 范围，不能仅把样例域名改成任意公网服务。

## 本地请求

一个 Unix 连接读取一行 JSON-RPC 请求并返回一行响应。上限256KiB、并发64；严格解码拒绝重复/未知键。以下命令读取节点状态（需要 Bun）：

```sh
bun -e '
import { createConnection } from "node:net";
const socket = createConnection("/tmp/conex-node-a/operator.sock");
socket.on("connect", () => socket.write(JSON.stringify({
  jsonrpc: "2.0", id: "01ARZ3NDEKTSV4RRFFQ69G5FAY", method: "node/snapshot",
  params: { context: { plane: "broker" }, input: {} }
}) + "\n"));
socket.on("data", chunk => { process.stdout.write(chunk); socket.end(); });
socket.on("error", error => { console.error(error.message); process.exitCode = 1; });
'
```

沿用同一信封替换 method/input：

| method | input | 结果 |
|---|---|---|
| `node/snapshot` | `{}` | epoch、实例/模块 generation、发布状态、非秘密资源状态、启动错误 |
| `node/start` | `{ "appId": "local-counter" }` | 仅能启动预装且完成校验的应用；已运行实例不会重复创建 |
| `node/invoke` | `{ "instanceId": "<snapshot实例ID>", "module": "counter", "generation": "<snapshot代际>", "method": "counter/read", "argument": {} }` | 同一领域契约的输入/授权/handler/输出校验 |
| `node/stop` | `{ "instanceId": "<实例ID>", "deadlineMs": 30000 }` | 缓存的 Stopped/Failed 报告与诊断；并发调用共享结果 |
| `node/shutdown` | `{}` | 逆序停止各应用、关闭节点发布和本地入口 |

`generation` 必须为规范正十进制字符串；`counter/add` 的 argument 为 `{ "delta": 1 }`。`deadlineMs` 不能延长30秒默认总截止；后续 stop 只能缩短已生效 deadline。SIGINT/SIGTERM 也执行显式关闭。

## 所有权与恢复

节点库为 `data_dir/domain.sqlite`，复用 P1 专用 SQLite 工作线程。v1→v2 使用 backup API 的一致快照后原子迁移，保留最近7份有效迁移快照；未知版本或坏库拒绝启动。node ID 绑定数据库；每次 boot 持久登记新 epoch，模块 generation 事务内单调。

模块持久键为 `SHA256(u64_be(app_id 字节长度) || app_id || module_id)`，避免斜杠拼接歧义与双64字节标识超限。自有资源在 spawn 前登记 Created，ready 后以 owner epoch 做 CAS 登记实际控制身份。控制令牌只在受限节点库与继承 stdin 中，不投影中心或节点 snapshot。

崩溃后先核对旧资源，再允许冲突模块启动：验证原控制通道、boot ID 与进程 start ticks，不凭 PID kill。无法确认退出则保留 Unknown 并拒绝该模块的新实例；其他应用仍可启动。外部 counter 只有连接引用，Node 不向它发 stop，也不 kill。不要为消除 Unknown 随意改库；先由实际资源创建者确认/处理资源。

boot 也执行 P1 `recover_unfinished`：可证未分派的 accepted 失败清槽，已越发送边界的执行保持 unknown/待核对，不重放旧命令。snapshot 的 `recoveryPendingAtBoot` 仅表示 boot 时待核对数量，不冒充 P3/P4 后端 reconciliation。

## 发布与故障

- 一个 Node 只有一个 Publication/中心连接，多应用各持租约；停一个应用不使其他应用换代。
- 中心断线只影响发布/该路径订阅；本地计数、进程、generation、node epoch 不变。重连重新发布已有服务，新 registrationId，旧引用 stale；不重放副作用。
- 应用先整体校验，再使用同一个稳定 Kahn 顺序启动。启动失败逆序回滚，原错误和 cleanup 错误同时保留；DB 记录失败不阻止其他资源清理。
- `Failed` 不等于进程仍活着，也不等于已清理完成；以报告和持久资源核对结果为准。Drop 不能代替异步 stop。

## 验收

```sh
cargo xtask e2e --suite private-runtime
cargo xtask check
```

第一条实际启动 Host、Node、自有计数进程和独立外部进程，验证断中心、重连、停止隔离、崩溃核对、不可达控制通道及所有权写入失败。总门禁已包含此套件。实际结果与未覆盖边界见 [P2 验证记录](../verification/private-runtime-p2.md)。
