import type { ImportFormat, RuleKind } from "@/lib/api";

export const KIND_LABEL: Record<RuleKind, string> = {
  suffix: "域名及子域名",
  domain: "仅此域名",
  keyword: "包含关键字",
  cidr: "IP / 网段",
};

export const KINDS: RuleKind[] = ["suffix", "domain", "keyword", "cidr"];

export const FORMAT_LABEL: Record<ImportFormat, string> = {
  auto: "自动识别",
  text: "文本（每行一条）",
  csv: "CSV / TSV",
  json: "JSON",
  yaml: "YAML（Clash rule-provider）",
};

/** An entry as one line of a text import, which keeps its kind. */
export function asLine(kind: RuleKind, value: string): string {
  switch (kind) {
    case "suffix":
      return `DOMAIN-SUFFIX,${value}`;
    case "domain":
      return `DOMAIN,${value}`;
    case "keyword":
      return `DOMAIN-KEYWORD,${value}`;
    case "cidr":
      return value;
  }
}

/** An address (v4 or v6), not a name. */
export function isAddress(host: string): boolean {
  return /^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.includes(":");
}
