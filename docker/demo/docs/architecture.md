# 架构速览

## 为什么要有中心 Host

分散在多台主机上的服务各自有网络边界。浏览器无法直连，运营方也不该为了
"让页面能用"而把内网端口暴露到公网。中心 Host 把这件事收敛成一个信任边界。

## 三段链路

```text
浏览器  ──HTTPS/WSS 同源──>  中心 Host  <══WSS 出站══  远端 Agent
                               │
                               ├─ 认证：静态 bearer / 匿名只读会话
                               ├─ 授权：租户 × 端点 × 资源范围
                               ├─ 投影：endpoint/list 只暴露有损信息
                               └─ 路由：把已校验的请求转给持有该端点的 Agent
```

关键点：**连接方向**。Agent 主动连 Host，内网不需要开任何入站端口。
这一条决定了多主机部署是否可行。

## 授权模型

授权在每次调用时判定，不在"登录成功"时判定。

- 租户来自认证上下文，永不信任请求自报
- 目录、读取、范围读取、have、提交、pin 走同一套资源授权
- 知道路径、持有 CID、upload 标识符或 pin ID 都不构成授权凭据
- Host 的判定优先于 Agent 自报的内容，Agent 侧再独立校验一次根目录范围

## 故障语义

一台主机掉线只影响它自己的端点：

- 该端点 `connectionState` 变为 `offline`，调用返回 `unavailable`
- 其余端点照常可读
- 页面不会自动重放失败调用，权限拒绝与离线是两种不同错误

## 协议

结构类型的唯一来源是 `schema/conex/*.proto`。Rust 类型、JSON Schema 与
TypeScript 类型都是生成产物，权威严格解码点在 `MethodContract.prepare` 与
`validate_output`。
