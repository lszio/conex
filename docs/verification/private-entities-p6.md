# P6 指标与关系 — 实施记录

日期:2026-11-01。范围:[P6 计划](../plans/2026-10-31-conex-mvp-p6.md) §2–§5。运行平台 macOS(darwin/arm64);**Linux 真实采样核对是本包 live gate,未执行**——按计划随 P9 两机验收,本机 smoke 只证明代码路径与量纲。

## 1. 交付内容

| 交付物 | 位置 | 证据类型 |
|---|---|---|
| 指标模块 | `crates/conex-agent/src/modules/metrics.rs`:真实 CPU(按相邻采样差,0..=100)/内存/磁盘字节采样,样本携带 sampledAtMs/platform/quality;环形缓冲有界(512);采样停止后投影转 stale(过期不删除);像普通模块提供 `conex.metrics/1`(`metrics/read`)并经正常发布 | 单元测试(collect 真实采集本机样本)+ e2e M1 |
| 关系与标注模块 | `crates/conex-agent/src/modules/relations.rs`:`conex.relations/1`(relations/list、tags/list、tags/put、tags/delete);描述关系从 store 投影(P3 relationships 表),能力绑定独立操作;Tags 是标注不是权限,删除只删标注行 | e2e M2 |
| proto 扩展 | `schema/conex/entities.proto`:MetricSample(kind/value/unit/platform/quality/sampledAtMs)、EntityTag(owner node/user)、MetricsRead*/TagsList*/TagsPut*/TagsDelete*;生成链同步(defs 208→217) | `cargo xtask generate --check` |
| store v5 | `entity_tags` 表(owner CHECK node/user、PK 幂等覆盖);record_tag/list_tags/delete_tag API;实体存在性校验 | domain 迁移链(v1→v5)+ e2e 往返 |
| 验收入口 | `xtask/src/private_entities_e2e.rs`(`cargo xtask e2e --suite private-entities`):真实节点 + 真实采样与本机 OS 数据源(sysinfo total_memory)量纲核对、标签往返、关系查询边界 | 见 §2 |

## 2. 实际运行结果(`cargo xtask e2e --suite private-entities`)

- M1 指标:样本 kind 集(cpu_percent/memory_total_bytes/…)、CPU 量纲 0..=100、字节数为正、sampledAtMs 新鲜(<120s)、quality=live;**memory_total_bytes 样本与本机 OS 总内存比值断言(0.5..=2.0)**。
- M2 标注:put → list(1 行,user 所有权)→ delete → list 空;node/user 所有权 CHECK 由 store 锁定。
- M3 关系:relations/list 对未知会话按契约拒绝(ULID 校验),不伪造空列表成功语义;backend 关系行由 P3 子代理生命周期写入。
- 断连/重连:采样循环停止 → STALE_AFTER(30s)窗口后投影转 stale(单元测试锁定窗口与字段);不把后端业务改 failed。

## 3. 明确未交付(不因本页关闭)

- **Linux 真实采样核对**:本包 live gate,归 P9 两机验收;macOS smoke 不替代。
- 能力绑定查询(谁经哪个契约使用哪个提供者)的独立授权语义:存储与操作面已分列,授权细化随 P7/P9 消费者接入。
- `environment.rs` 组合摘要模块:归 P9(计划 §1 明确不预建)。

## 4. 门禁状态

`cargo test -p conex-agent`(47,含 P6 模块与采集测试)全绿;e2e 套件结果见 §2;全量门禁随部署收尾汇报。未提交/推送/部署。
