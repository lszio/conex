// 私有会话的浏览器侧持久化：cookie 归浏览器，CSRF 是"写"凭据，必须随
// 会话保存。放 localStorage（而非 sessionStorage）是因为 cookie 是浏览器级
// 的：新标签页要在不重新引导码的情况下恢复同一会话的写能力。明文 CSRF 只
// 对已能读该存储的脚本有意义——那类脚本已经赢了，存储选择不改变威胁面。

import type { PrivateHubClient } from "@conex/sdk";

const CSRF_KEY = "conex.private.csrf.v1";

export function restoreCsrf(hub: PrivateHubClient): void {
  try {
    const saved = localStorage.getItem(CSRF_KEY);
    if (saved) hub.csrf = saved;
  } catch {
    // 隐私模式等场景下存储不可用：写操作会在下次登录前被服务端拒绝。
  }
}

export function saveCsrf(hub: PrivateHubClient): void {
  try {
    if (hub.csrf) localStorage.setItem(CSRF_KEY, hub.csrf);
  } catch {
    // 同上：不因存储失败把登录也打断。
  }
}

/** 节点直连地址按 nodeId 记忆；hub 不下发表达式地址（P5：已知地址直连）。 */
export function nodeAddressKey(nodeId: string): string {
  return `conex.node.address.${nodeId}`;
}

export function loadNodeAddress(nodeId: string): string {
  try {
    return localStorage.getItem(nodeAddressKey(nodeId)) ?? "http://127.0.0.1:7451";
  } catch {
    return "http://127.0.0.1:7451";
  }
}

export function saveNodeAddress(nodeId: string, address: string): void {
  try {
    localStorage.setItem(nodeAddressKey(nodeId), address);
  } catch {
    // 不可持久化时仍可当次会话使用。
  }
}
