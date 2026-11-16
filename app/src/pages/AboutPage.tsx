import { CalendarRange, Feather, Radar, RotateCcw } from "lucide-react";
import appIcon from "@/assets/app-icon.png";
import { Group, Page } from "@/components/PageHeader";
import type { AppInfo } from "@/lib/api";

const points = [
  { icon: Radar, title: "直接读取已有的内核", text: "自动找到 Clash Verge Rev 等客户端正在运行的 mihomo 内核，只读取流量计数，不接管、不改动代理。" },
  { icon: CalendarRange, title: "像 vnstat 一样记账", text: "按 15 分钟记录、永久保存，按本地时间汇总成小时、日、周、月、年。" },
  { icon: RotateCcw, title: "不漏记", text: "以内核的累计计数为准；Mihomo-Monitor 退出或电脑睡眠期间，只要内核没有重启，流量会在下次读取时补回。" },
  { icon: Feather, title: "常驻托盘，几乎不占资源", text: "后台统计只是一个轻量的 Rust 进程；开启轻量模式时，关闭窗口会连界面一起释放。" },
];

export function AboutPage({ info }: { info: AppInfo | null }) {
  return (
    <Page>
      <h1 className="sr-only">关于</h1>
      <div className="space-y-5 px-5 pt-3">
        <div className="flex flex-col items-center py-4 text-center">
          <img src={appIcon} alt="" className="h-16 w-16 rounded-[18px]" draggable={false} />
          <div className="mt-3 text-[17px] font-semibold tracking-[-0.01em]">Mihomo-Monitor</div>
          <div className="mt-0.5 text-[13px] text-muted">mihomo 内核的流量统计</div>
          <div className="mt-2.5 inline-flex h-[22px] items-center rounded-full bg-card px-2.5 font-mono text-[11px] text-fg-2 shadow-card">
            {info ? `v${info.version} · ${info.platform}` : " "}
          </div>
        </div>

        <Group>
          {points.map((p) => (
            <div key={p.title} className="flex items-start gap-3 px-4 py-3">
              <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-pill text-fg-2">
                <p.icon className="h-[15px] w-[15px]" />
              </span>
              <div className="min-w-0">
                <div className="text-[13px] font-medium">{p.title}</div>
                <p className="mt-0.5 text-[12px] leading-[18px] text-muted">{p.text}</p>
              </div>
            </div>
          ))}
        </Group>
      </div>
    </Page>
  );
}
