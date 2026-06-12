// Anytype API Key 认证管理
//
// 配置来源（优先级高→低）:
//   1. 环境变量: ANYTYPE_API_KEY, ANYTYPE_API_BASE_URL, ANYTYPE_API_VERSION
//   2. YAML 配置文件: ~/.conex/config.yaml
//   3. 硬编码默认值
//
// YAML 配置格式 (~/.conex/config.yaml):
//   anytype:
//     apiKey: "your-key"
//     apiBaseUrl: "http://127.0.0.1:31009"
//     apiVersion: "2025-11-08"

import { readFileSync, writeFileSync, existsSync, mkdirSync } from "fs";
import { homedir } from "os";
import { join } from "path";

const CONEX_DIR = join(homedir(), ".conex");
const CONFIG_FILE = join(CONEX_DIR, "config.yaml");

export interface AnytypeCredentials {
  apiKey: string;
  apiBaseUrl: string;
  apiVersion: string;
}

/** 硬编码默认值 */
function defaults(): AnytypeCredentials {
  return {
    apiKey: process.env.ANYTYPE_API_KEY || "",
    apiBaseUrl: process.env.ANYTYPE_API_BASE_URL || "http://127.0.0.1:31009",
    apiVersion: process.env.ANYTYPE_API_VERSION || "2025-11-08",
  };
}

/** 动态 import yaml（仅在文件存在时） */
let _yamlParse: ((s: string) => unknown) | null = null;
let _yamlStringify: ((v: unknown) => string) | null = null;

async function ensureYaml() {
  if (_yamlParse) return;
  const mod = await import("yaml");
  _yamlParse = mod.parse;
  _yamlStringify = mod.stringify;
}

async function loadYamlFile(): Promise<Record<string, unknown> | null> {
  if (!existsSync(CONFIG_FILE)) return null;
  try {
    await ensureYaml();
    const raw = readFileSync(CONFIG_FILE, "utf-8");
    const parsed = _yamlParse!(raw);
    if (parsed && typeof parsed === "object") return parsed as Record<string, unknown>;
    return null;
  } catch {
    return null;
  }
}

/** 加载凭证: 环境变量 → YAML 文件 → 默认值 */
export async function loadCredentials(): Promise<AnytypeCredentials> {
  const env = defaults();
  if (env.apiKey) return env;

  const parsed = await loadYamlFile();
  if (parsed) {
    const anytype = parsed.anytype as Record<string, unknown> | undefined;
    if (anytype && typeof anytype === "object") {
      return {
        apiKey: String(anytype.apiKey ?? env.apiKey),
        apiBaseUrl: String(anytype.apiBaseUrl ?? env.apiBaseUrl),
        apiVersion: String(anytype.apiVersion ?? env.apiVersion),
      };
    }
  }

  return env;
}

/** 保存凭证到 ~/.conex/config.yaml */
export async function saveCredentials(creds: Partial<AnytypeCredentials>): Promise<void> {
  mkdirSync(CONEX_DIR, { recursive: true });
  await ensureYaml();

  let existing: Record<string, unknown> = {};
  const raw = (await loadYamlFile()) || {};

  const anytype = { ...(raw.anytype as Record<string, unknown> || {}), ...creds };
  const merged = { ...raw, anytype };
  writeFileSync(CONFIG_FILE, _yamlStringify!(merged), "utf-8");
}

/** 检查是否有可用的 API Key */
export async function hasApiKey(): Promise<boolean> {
  if (process.env.ANYTYPE_API_KEY) return true;
  const parsed = await loadYamlFile();
  if (!parsed) return false;
  const anytype = parsed.anytype as Record<string, unknown> | undefined;
  return !!(anytype?.apiKey);
}