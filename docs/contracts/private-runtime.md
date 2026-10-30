# Conex 私有运行与安全契约

日期：2026-10-29。状态：P2 自治节点/应用包装配/本地受限入口已实现，P5 网络授权仍为规范。依据：[架构基线](../design/2026-10-29-conex-private-platform.md)、[P0](../plans/2026-10-29-conex-mvp-p0.md)、[Agent](agent-domain.md)、[事件恢复](event-recovery.md)；当前实际证据见 [P2 记录](../verification/private-runtime-p2.md)。

证据用语：**源码观察**不等于运行验证；**规范决策**是后续实现输入。适配器协议调查见 [探针记录](../verification/agent-protocol-probes.md)，自治真实进程验证见 [P2](../verification/private-runtime-p2.md)。本契约没有证明公网授权/Web 直连已交付。

## 1. Scope、所有权与生命周期

Scope 是本地所有权/清理/访问边界，不是一棵跨机器关闭树。客户端只拥有自己的订阅/连接，组合只拥有它实际创建的实例；消费远端能力不拥有远端进程。外部 Codex app-server 只能断连接，不能 kill。

生命周期严格分开：

- 本地实例：Installed → Starting → Active → Stopping → Stopped；失败进入 Failed。按本地依赖拓扑启动、逆拓扑停止/回滚，停止先拒绝新增操作，截止时间默认30秒；仅终止已确认归属的自有资源。
- 发布：Unpublished → Registering → Published → Disconnected → Registering → Published（新 registrationId）。中心失联仅停止该发布路径/远程订阅，不关闭本地业务。
- 连接：Disconnected → Connecting → Authenticating → Ready → Reconnecting。源码观察：现有节点心跳15秒、退避1–30秒；属于连接策略，不是业务存活租约。

**三种身份不可混用**：nodeRuntimeEpoch 为节点每次进程启动时产生并持久登记的ULID；module generation 为模块实例代际（节点持久计数）；registrationId/link generation 为中心每次发布/连接代次。中心重连不能改变前两者，也不能使仍有效直连授权失效。节点重启改变 nodeRuntimeEpoch，模块重启改变其 generation，旧引用明确失效。

自有子进程不一定随父进程崩溃退出。节点持久记录创建所有权、原运行epoch和控制通道身份，重启后先核对旧资源；不凭一个PID判断归属、不直接杀复用PID，不把未知旧进程当已停止而并行重建冲突实例。只有明确退出或完成受控接管后才重建；外部对象永远不转成自有。

P2 具体落地：Node 单一有界协调器持有应用/资源；同一 Node 的应用共享一个 Publication/中心连接，各应用借用可撤销发布租约。持久 moduleKey 用 `SHA256(u64_be(app_id字节长度) || app_id || module_id)` 编码，避免分隔符碰撞和标识组合超限。资源 spawn 前先登记控制通道保留项，ready 后按 owner epoch CAS；不能证明退出的项维持 unknown 并阻止冲突启动。

## 2. 应用包：定义与环境

应用包是可信管理者安装的定义数据，不是授权来源。节点静态实现注册表提供真正工厂与实现成员；包不能下载代码、指定任意执行命令或把自报principal当身份。示例名字只用于本地装配，wire Node/Principal等使用注册的ULID身份。

```toml
schema = "conex.application/1"
app_id = "workbench"
version = "0.1.0"
owner_node_ref = "home-node"

[[modules]]
id = "agent"
implementation = "omp"
config_ref = "omp-default"
permissions = ["agent.read", "agent.execute", "agent.interact"]

[[modules]]
id = "metrics"
implementation = "linux-metrics"

[[modules]]
id = "summary"
implementation = "environment-summary"
local_dependencies = { agent = "agent", metrics = "metrics" }
exports = [{ contract_id = "conex.environment/1", strategy = "direct-only" }]

[[remote_bindings]]
name = "other-agent"
node_ref = "second-node"
provider_endpoint_ref = "agent"
contract_id = "conex.agent/1"
```

未知/重复字段拒绝。config_ref 仅引用节点批准配置表，包本身不能给env/file任意路径。节点环境负责可执行文件、Workspace根、socket地址和凭据引用；单纯出现 `file:` 不等于安全授权。owner_node_ref 必须匹配安装目标，安装操作者来自可信本地配置/认证上下文。

校验顺序：schema版本、标识/大小、实现允许表、config_ref允许表、模块/依赖名重复、本地依赖缺失/环、远程绑定完整性、permissions不超节点上限、导出成员实际存在、来源数据策略。上限：包256KiB、模块64、每模块依赖64、远程绑定64；本地Kahn顺序用稳定模块名打破并列。远程绑定不参与本地环/启动事务；失效按声明降级，不拖垮无关模块。

装配错误：InvalidPackage、UnknownImplementation、ConfigRefNotAllowed、DuplicateModule、DuplicateBinding、MissingDependency、CircularDependency、GrantNotAllowed、ExportNotImplemented、ExportNotAllowed、OwnerMismatch。不能以缺实现为由创建空模块返回成功。配置错误/启动失败逆序清理已创建资源，保留原错误与清理诊断。

P2 当前安装的实际实现只有 `controlled-counter` / `external-counter`，并非上面 Agent/指标目标示例已实现。非秘密可运行包见 [节点样例](../../examples/mvp/node/application.toml)；实际工厂接收显式 `ModuleRuntime`（Store/epoch/generation/资源身份）和 SDK `ModuleContext`，不通过全局隐式状态取得所有权。LocalApi 使用受限 Unix socket + OS peer UID，不开放匿名 loopback HTTP。

## 3. 主体、凭据与授权格式

### 3.1 接入凭据

节点→中心：专用node接入bearer，WSS upgrade Authorization头；中心存摘要，节点经批准的env/file后端读取。Emacs→中心：独立客户端bearer，auth-source提供；不读浏览器cookie。接入身份与具体能力授权分开，不能因为握手成功就访问全部模块。

浏览器→中心：私有认证建立HttpOnly/Secure/SameSite=Strict cookie；默认拒绝匿名，private.enabled与历史web_guest互斥。复用当前web_auth的会话/CSRF/ticket能力，但新增私有登录引导必须由本机管理员一次性引导码完成；不得把现有访客会话当已认证用户。引导码随机256bit、5分钟、单次消费，仅管理员本机命令产生；只在HTTPS POST正文提交、不进URL/日志。中心重启会话失效需重新认证，不自动变访客。

### 3.2 授权 grant（规范决策）

grant为标准compact JWS JWT，**EdDSA/Ed25519**；使用已存在的ring维护的签名/验证原语，不自制算法。不使用跨节点共享HS256秘密：中心只持私钥，节点仅持批准issuer的公钥，受控节点无法为其他节点签发授权。header严格固定alg=EdDSA、typ=JWT、kid为安装密钥ID；未知kid/alg、重复键、非规范base64url、超过16384bytes token拒绝。验签成功后才使用claim，不从未验证字段选择任意网络JWKS。

必填claim：iss、aud（目标nodeId）、sub（认证Principal）、tenant（当前私有实例标识）、nodeEpoch、scope、iat、nbf、exp、jti。时间为JWT标准整秒JSON整数，须在安全整数范围并验证iat<=nbf<=exp；**这是JWT格式，不是Conex protobuf uint64 JSON规则**。scope使用生成GrantScope的camelCase映射：`{spaceId,providerEndpointId,contractId,moduleGeneration,methods,states,workspaceIds,conversationIds,purpose}`。ID与范围来自中心批准及节点当前发布；methods/states精确匹配，集合不是“交集非空就允许整个请求”。每次操作要求请求全部目标/资源都在授权与节点上限交集中。purpose=direct/relay防止换路径扩权。

nodeEpoch来自该节点已认证发布的运行epoch，而非中心AgentRegistry连接代次。moduleGeneration来自实际模块实例。jti是唯一授权ID（256bit随机），**不是一次性调用ID**。grant在有效期内可用于多次授权操作/重连，幂等仍由领域操作负责；一次性语义仅属于WSS票据。

私钥存中心受限文件、通过凭据引用装载；节点公钥/issuer/kid由管理员固定配置。轮换先向节点安装新公钥、中心切新kid签发、旧钥保留至其最后授权exp后60秒；紧急本地删除kid立即拒绝。公钥不能通过未经验证的节点请求更换。

### 3.3 有效期、撤销与边界

默认relay grant 5分钟、direct grant 1小时；签发不得超对应上限。nbf最多容忍60秒时钟偏差，exp为硬截止，不在过期后另加容差延长离线窗口。时钟回拨导致时间不可信时拒绝续准入，要求核对，不无限延续已过期授权。

每个调用/补读/订阅更新检查到期、kid、nodeEpoch、moduleGeneration、撤权和精确资源范围；已有WSS通过一次升级不等于之后永久获权。到期/本地撤权结束受影响订阅、拒绝新命令，不自动取消业务执行或回滚已发送副作用。普通操作超时不使执行停止。

本地撤权必须先持久提交节点store，再撤销内存授权/队列并回执；重启不丢失本地主体/密钥撤权。中心撤销以单调revision日志同步，节点持久记录水位，重连从水位分页同步；同revision重复幂等、缺口要求完整快照，不能用毫秒timestamp作为唯一游标漏同刻撤销。中心分区撤销不保证即时，最迟在grant.exp拒绝新访问；本地已知撤销立即执行。

一次性ticket消费表上限4096、仅保留到ticket.exp；满则拒绝签新票，不驱逐有效票来冒充重放防护。grant本身不进入“已消费”表。

## 4. Web/Emacs直连与票据

浏览器中心会话复用当前30秒一次性WSS票/Origin绑定的模式。ticket与bearer不同，不得把grant改名ticket塞入URL。规范MVP票据交付：HTTPS POST获取后，用WSS子协议 `conex-ticket.<opaque>` 提交（同时请求实际Conex profile）；网关与服务端禁止记录该子协议值。服务端返回实际profile，不回显票据。源码现有URL票据路径是迁移输入，不可直接声称已符合新私有边界。

浏览器直连：节点配置HTTPS/WSS证书、精确允许的Web Origin及批准公开地址；客户端只访问获准目录地址，不能传任意私网URL。使用fetch `POST <node>/private/tickets`，Authorization头提交direct grant，Origin必须精确批准；节点返回自身30秒一次性ticket，再以相同子协议升级。允许的CORS仅精确Origin、POST和Authorization/Content-Type，不用*、不接受null Origin、不用节点跨站cookie。浏览器私网访问限制/证书问题是实际P5/P9验收项，不声称任何网络浏览器均可直连。私网可达且受浏览器策略允许时必须实现Web直连；不能静默把该能力缩成仅native。

native/Emacs直接用Authorization: Bearer grant升级，无浏览器cookie。中心申请grant接口同时支持已认证专用客户端bearer，不能只支持cookie/CSRF而阻断Emacs；cookie路径必须Origin+CSRF，native bearer路径不用浏览器CSRF但检查role/目标权限。

私有登录、grant申请、revoke、tickets在请求体/头中传秘密，不进入URL、buffer、kill-ring、目录或诊断正文。多端共享服务端会话与审批状态；草稿等本地状态独立。

## 5. 导出路径与来源数据

导出策略direct-only/relay-allowed，默认direct-only。目录只保存管理员批准的非敏感元数据（ID/契约/获准地址/能力说明），不自动同步私网路径、prompt、正文、敏感标签；direct-only的调用、head、快照、事件页、同步/内容路由都必须拒绝经中心读取或落盘。

派生数据出口默认取**所有来源策略的交集/最严格者**；包含direct-only来源的摘要也默认direct-only。模块声明permissions只表示申请，不能靠增加grants字符串洗掉来源限制。允许对特定数据降密必须有管理员独立批准的`{sourceRef,fieldSet,destination,allowedStrategy}`，来源节点与中心双端核对；首版没有该明确批准就拒绝ExportNotAllowed。仅做精确发布元组匹配不能证明数据流没有泄漏；可信模块负责按批准构建投影，不把普通进程内模块当信息流沙箱。

中心只按已注册Conex endpoint路由，不接受任意URL/TCP端口代理。直连与转发使用相同领域方法/授权上限；传输中断不能自动换路重发副作用。取消固定原执行/实例，未知结果继续unknown。

## 6. 持久化、备份与崩溃

驱动统一使用rusqlite `=0.41.1`，MIT，default-features=false、features=[bundled,backup]；[发布元数据](https://crates.io/api/v1/crates/rusqlite/0.41.1)。节点库已在P1/P2落地，协调端private_store仍待P5。两库独立，不共享跨机器SQLite：中心private_store保存目录/配置/授权ledger与允许的标注；节点domain store保存映射/投影/事件/运行代际/所有权/本地撤销水位。

DB目录0700、DB/WAL/SHM/备份0600；阻塞操作放专用工作线程/有界队列，禁止异步锁跨网络。WAL、foreign_keys=ON、synchronous=FULL；迁移user_version单调、事务失败回滚，程序不认识的新版本或integrity_check失败必须拒绝启动，不用坏库继续“成功”。不回退内存存储。

迁移前用SQLite backup API复制一致快照，保留最近7份；不直接拷贝活跃WAL主文件，不把节点库备份到中心。中心备份禁止正文/凭据；节点授权恢复数据库备份**可能含本机正文**，必须同样受限存放并遵守保留/删除政策，不能承诺“所有备份无正文”。私钥文件不打包进数据库备份。

中心不持久化对话正文/审批秘密/凭据，禁止经日志、错误dump、公共缓存、通用stream replay绕存。P5/P9以随机测试正文/秘密canary扫描DB、WAL、备份和日志；认证ledger可以保存jti/主体/权限范围，不能保存原grant字符串或私钥。节点崩溃后执行恢复遵守Agent发送边界：accepted可证未分派则失败清槽，dispatching及之后先unknown核对；不恢复旧命令发送。

## 7. 负路径与真实验收

| ID | 场景 | 必须结果 |
|---|---|---|
| N01 | 匿名私有WSS/RPC/目录 | 401，无访客会话/目录泄漏 |
| N02 | 错误节点/客户端凭据；伪造principal | 401或严格wire拒绝，不调用业务 |
| N03 | grant过期、nbf未到、issuer/kid/aud不符 | 拒绝，不能靠WSS长连绕开exp |
| N04 | 同grant多次合法操作；同ticket二次升级 | 前者允许至有效期，后者只首次成功 |
| N05 | 错Origin/null Origin/任意目标URL | 升级/取票拒绝，不做代理 |
| N06 | payload自报AccessContext/tenant | 拒绝，不参与真实Caller构造 |
| N07 | scope仅部分匹配或资源不在范围 | forbidden，不以交集非空放行 |
| N08 | direct grant用于relay，或反向 | forbidden，路径不扩权 |
| N09 | direct-only经invoke/head/read/blob/同步 | 全拒绝，中心无正文写盘 |
| N10 | 聚合声明自授予来源降密 | 装载/发布拒绝，需独立管理批准 |
| N11 | 本地撤权后新读/更新及节点重启 | 立即拒绝且撤权持久，不自动取消执行 |
| N12 | 中心分区/同刻多条撤销 | 有效直连可继续至exp；revision同步无漏项 |
| N13 | 慢消费者/票据表或队列满 | 明确拒绝/end/resync，无无界缓存 |
| N14 | 停中心并恢复 | 业务/有效直连继续；仅发布换代，不撤销直连nodeEpoch |
| N15 | 节点/模块实际重启 | 旧nodeEpoch/moduleGeneration失效，未知执行先核对 |
| N16 | Web与Emacs各申请grant并直连 | cookie+CSRF与专用bearer均可；无URL秘密 |

P2 已真实验证所有权/断中心/重连和本地伪造principal拒绝；不等于本表N01–N16全通过。P5负责真实loopback进程授权、双路径与浏览器Origin，P9负责公网入口→两机私网、真实Web/Emacs及canary。部署、TLS现网变更仍需独立授权，不因本契约而开放公网。
