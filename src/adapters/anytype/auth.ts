// Anytype API Key 认证管理
// API Key 可以从 Anytype 应用 -> 设置 -> API Keys -> Create new 获取

import { mkdirSync, readFileSync, writeFileSync, existsSync } from "fs";
import { homedir } from "os";
import { join } from "path";

const CONEX_DIR = join(homedir(), ".conex");
const CREDENTIALS_FILE = join(CONEX_DIR, "credentials.json");

export interface AnytypeCredentials {
  apiKey: string;
  apiBaseUrl: string;
  apiVersion: string;
}

/** 默认 Anytype API 连接配置 */
function defaults(): AnytypeCredentials {
  return {
    apiKey: process.env.ANYTYPE_API_KEY || "",
    apiBaseUrl: process.env.ANYTYPE_API_BASE_URL || "http://127.0.0.1:31009",
    apiVersion: "2025-11-08",
  };
}

/** 从环境变量或凭证文件加载 API Key */
export function loadCredentials(): AnytypeCredentials {
  const creds = defaults();

  // 环境变量优先
  if (creds.apiKey) return creds;

  // 尝试从凭证文件读取
  try {
    if (existsSync(CREDENTIALS_FILE)) {
      const saved = JSON.parse(
        readFileSync(CREDENTIALS_FILE, "utf-8"),
      ) as Partial<AnytypeCredentials>;
      return { ...creds, ...saved };
    }
  } catch {
    // 忽略文件读取错误
  }

  return creds;
}

/** 保存凭证到文件 */
export function saveCredentials(creds: Partial<AnytypeCredentials>): void {
  mkdirSync(CONEX_DIR, { recursive: true });

  const existing: Partial<AnytypeCredentials> = existsSync(CREDENTIALS_FILE)
    ? JSON.parse(readFileSync(CREDENTIALS_FILE, "utf-8"))
    : {};

  writeFileSync(
    CREDENTIALS_FILE,
    JSON.stringify({ ...existing, ...creds }, null, 2),
    "utf-8",
  );
}

/** 检查是否有可用的 API Key */
export function hasApiKey(): boolean {
  const creds = loadCredentials();
  return creds.apiKey.length > 0;
}