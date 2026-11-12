# 设计文档索引

本目录保留 Pinglet 各阶段的设计稿、实现计划与验收记录。设计稿反映当时的决策过程，**若与当前代码或 [使用说明](../usage.md) 不一致，以代码与使用说明为准**。

## 当前基线

| 文档 | 内容 | 状态 |
| --- | --- | --- |
| [pinglet-v1.2.md](pinglet-v1.2.md) | 四态模型、UI 布局、主题、设置面板、托盘 / 快捷键 / 边缘吸附的设计定稿 | 已实现，仍是状态与交互的基线 |
| [pinglet-v1.3.md](pinglet-v1.3.md) | 窗口尺寸动态调整等增量改造 | 已实现（默认窗口高度最终为 320px） |
| [pinglet-v1.4.md](pinglet-v1.4.md) | 托盘三态动画、自动主题、自绘通知弹窗、单实例、免打扰、窗口位置记忆 | 已实现于 v1.4.0，含多轮真机验收记录 |
| [pinglet-v1.4.1.md](pinglet-v1.4.1.md) | 通知停留时长、logo 常亮、托盘动画、临时隐藏、显隐动画、主题过渡 | 已实现于 v1.4.1 |

## 历史记录

| 文档 | 内容 | 状态 |
| --- | --- | --- |
| [pinglet.md](pinglet.md) | 项目背景、目标、技术选型与首版实现计划 | 历史背景；UI 与功能以 v1.2 定稿为准 |
| [pinglet-logo.md](pinglet-logo.md) | Logo 静态状态映射与视觉规范 | 已应用；其中置灰 / 高亮联动已在 v1.4.1 改为常亮 |
| [pinglet-repo-migration.md](pinglet-repo-migration.md) | 从 `claude-skill-lab` 迁出为独立仓库 | 已执行（2026-11-07） |

## 相关文档

- [使用说明](../usage.md)
- [构建与打包](../build.md)
- [会话标题数据源](../session-title-source.md)
- [优化待办](../optimization-backlog.md)
- [变更日志](../../CHANGELOG.md)
