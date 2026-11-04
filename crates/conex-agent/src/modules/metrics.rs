//! 服务器指标采集（P6.1）：真实 OS 数据、CPU 按相邻采样差计算百分比、
//! 每样本携带时间与平台；采样循环停止后样本投影转 stale（过期而非删除）。
//! 保留有界（环形缓冲）；不支持的指标显式省略，不填 0 冒充。
//!
//! Linux 真实采样核对是本包 live gate；本机 smoke 只证明代码路径与量纲
//! （P6 计划 §2），Linux 核对随 P9 两机验收执行。
use crate::adapters::common::stable_ulid;
use crate::node_types::RuntimeFactory;
use conex_agent_domain::Store;
use conex_proto as p;
use conex_sdk::{Contract, Execution, Method, StartedModule};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, watch};

pub const CONTRACT_ID: &str = "conex.metrics";
const RING_CAPACITY: usize = 512;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
/// 投影过期窗口：样本 older than this 转 stale（P6 计划 §5）。
const STALE_AFTER_MS: u64 = 30_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

struct SampleRing {
    samples: Mutex<VecDeque<p::MetricSample>>,
}

/// 一次真实采集。CPU 百分比按两次刷新差计算（sysinfo global usage 需要间
/// 隔）；内存/磁盘为绝对字节。磁盘不可用时省略，不填 0。
async fn collect(platform: &str) -> Vec<p::MetricSample> {
    let now = now_ms();
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    tokio::time::sleep(Duration::from_millis(400)).await;
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let mut samples = Vec::new();
    let cpu = sys.global_cpu_usage();
    if cpu.is_finite() && (0.0..=100.0).contains(&cpu) {
        samples.push(p::MetricSample {
            kind: "cpu_percent".into(),
            value: cpu as f64,
            unit: "percent".into(),
            platform: platform.into(),
            quality: "live".into(),
            sampled_at_ms: now,
        });
    }
    let total = sys.total_memory();
    if total > 0 {
        samples.push(p::MetricSample {
            kind: "memory_used_bytes".into(),
            value: sys.used_memory() as f64,
            unit: "bytes".into(),
            platform: platform.into(),
            quality: "live".into(),
            sampled_at_ms: now,
        });
        samples.push(p::MetricSample {
            kind: "memory_total_bytes".into(),
            value: total as f64,
            unit: "bytes".into(),
            platform: platform.into(),
            quality: "live".into(),
            sampled_at_ms: now,
        });
    }
    let disks = sysinfo::Disks::new_with_refreshed_list();
    for disk in disks.list() {
        if disk.mount_point().to_string_lossy() == "/" {
            samples.push(p::MetricSample {
                kind: "disk_total_bytes".into(),
                value: disk.total_space() as f64,
                unit: "bytes".into(),
                platform: platform.into(),
                quality: "live".into(),
                sampled_at_ms: now,
            });
            samples.push(p::MetricSample {
                kind: "disk_available_bytes".into(),
                value: disk.available_space() as f64,
                unit: "bytes".into(),
                platform: platform.into(),
                quality: "live".into(),
                sampled_at_ms: now,
            });
            break;
        }
    }
    samples
}

pub fn factory(_config: crate::node_config::MetricsModuleConfig) -> RuntimeFactory {
    Arc::new(move |runtime, _context| {
        Box::pin(async move {
            eprintln!("conex-agent metrics: factory enter");
            let platform = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
            let ring = Arc::new(SampleRing {
                samples: Mutex::new(VecDeque::new()),
            });
            let (stop_tx, stop_rx) = watch::channel(false);
            {
                let ring = ring.clone();
                let sampler_platform = platform.clone();
                tokio::spawn(async move {
                    let mut stop = stop_rx;
                    loop {
                        if *stop.borrow() {
                            break;
                        }
                        for sample in collect(&sampler_platform).await {
                            let mut ring = ring.samples.lock().await;
                            if ring.len() >= RING_CAPACITY {
                                ring.pop_front();
                            }
                            ring.push_back(sample);
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(SAMPLE_INTERVAL) => {}
                            _ = stop.changed() => break,
                        }
                    }
                });
            }
            eprintln!("conex-agent metrics: putting endpoint");
            let store: Store = runtime.store.clone();
            store
                .put_endpoint(p::AgentAdapterEndpoint {
                    id: stable_ulid(&format!(
                        "endpoint:{}:{}",
                        runtime.node_epoch, runtime.module_key
                    )),
                    node_id: runtime.node_id.clone(),
                    module_instance_id: runtime.resource_id.clone(),
                    generation: runtime.generation,
                    adapter: "builtin".into(),
                    backend_version: platform,
                    protocol_version: CONTRACT_ID.into(),
                    experimental: false,
                    capabilities: None,
                    availability: "ready".into(),
                    catalog_scope: "node-managed".into(),
                    limits: None,
                    display_name: Some("Server metrics".into()),
                })
                .await
                .map_err(|error| crate::node::failure(&error.to_string()))?;
            let read = {
                let ring = ring.clone();
                Method::new(
                    "conex.MetricsReadInput",
                    "conex.MetricsReadResult",
                    Execution::ReadOnly,
                )
                .with_handler(move |_context, _input: p::MetricsReadInput| {
                    let ring = ring.clone();
                    async move {
                        let now = now_ms();
                        let mut ring = ring.samples.lock().await;
                        for sample in ring.iter_mut() {
                            if sample.quality == "live"
                                && now.saturating_sub(sample.sampled_at_ms) > STALE_AFTER_MS
                            {
                                sample.quality = "stale".into();
                            }
                        }
                        let items = ring
                            .iter()
                            .map(|sample| {
                                json!({
                                    "kind": sample.kind,
                                    "value": sample.value,
                                    "unit": sample.unit,
                                    "platform": sample.platform,
                                    "quality": sample.quality,
                                    "sampledAtMs": sample.sampled_at_ms.to_string(),
                                })
                            })
                            .collect::<Vec<_>>();
                        Ok(json!({ "items": items }))
                    }
                })
            };
            eprintln!("conex-agent metrics: building contract");
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([("metrics/read".to_string(), read)]),
                BTreeMap::new(),
            )?);
            let mut started = StartedModule::new(contract);
            started.cleanup = Some(Arc::new(move |_deadline| {
                let _ = stop_tx.send_replace(true);
                Box::pin(async move { Ok(()) })
            }));
            Ok(started)
        })
    })
}

/// 指标形状守卫：样本必须带时间/平台/质量，CPU 量纲 0..=100（P6 计划 §2）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_capacity_is_bounded() {
        const _: () = assert!(RING_CAPACITY <= 4096, "metric retention must stay bounded");
    }

    #[tokio::test]
    async fn collect_produces_typed_live_samples() {
        let samples = collect("test/arch").await;
        assert!(
            !samples.is_empty(),
            "collect must produce samples on this host"
        );
        for sample in &samples {
            assert!(sample.sampled_at_ms > 0);
            assert_eq!(sample.quality, "live");
            assert_eq!(sample.platform, "test/arch");
            if sample.kind == "cpu_percent" {
                assert!(
                    (0.0..=100.0).contains(&sample.value),
                    "cpu sample {sample:?}"
                );
            }
        }
        assert!(samples.iter().any(|s| s.kind == "memory_total_bytes"));
    }
}
