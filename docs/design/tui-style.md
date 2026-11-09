# TUI style：统一的阅读与工作层次

更新日期：2026-11-09。独立产品为 Rous；下文带日期的 Saki 验收属于历史构建。显示行为和原有配置名保留，执行归属见 [Rous 边界](tui.md)。

状态：实验已实现；2026-10-15 起主题检测改为 Termina 原生输入 + OSC 10/11 采样（tmux 元数据路径已移除，历史验收见下方）。仍不声称物理 Ghostty 交互、OS 剪贴板或真实模型生成已验证。本文已按实际实现修订；历史设计判断保留在对应小节。

依据是用户实测截图、当前 renderer 和 [Pi／Codex 显示研究](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-presentation.md)。[现有 TUI](tui.md) 继续记录已实现行为。所有产品文案使用英文。`previews/tui-style.html` 是交互设计示意，内容与执行记录为样例，不构成新的项目统计或真实执行证据。

## 实验实现：已支持的行为

2026-10-14 在 `src/frontend/tui/` 实现下列行为。主题在 `theme.rs`，Markdown 解析在 `markdown.rs`，Markdown 布局在 `markdown_layout.rs`，Activity 状态与投影在 `state.rs` 和 `view.rs`；没有修改 Agent、协议、SessionManager、JSONL 或 console 语义。

### 主题

- `theme.rs` 定义成对角色：text、secondary、accent、rule、user_fg/user_bg、code、warning、error。默认 `SAKI_THEME=auto`：Termina `EventReader`/`EventStream` 是唯一输入路径；启动时发送 OSC 10/11、启用 mode2031 并用 CSI ?996 建立 scheme baseline，运行期 CSI ?997 变化只刷新 OSC 10/11。正文始终 `Color::Reset` 继承终端前景，user surface/secondary/accent/code/warning/error 从采样 fg/bg 推导并保持 ≥4.5 对比度；未知/`rgba:`/不可读时回退 native。手动 `SAKI_THEME=light|dark` 固定且不发送查询、不启用订阅，未知值静默回 auto。
- `App.theme` 是唯一渲染调色板：view、help、命令补全、会话选择器、日志、输入、状态栏与选择高亮都从它读取；环境变量只在 `tui::run` 启动时解析一次，之后由检测结果或手动设置更新。
- 用户矩形仍为低对比底色配正文色；选择高亮使用 accent 背景与 user surface 前景。单元测试验证成对角色对比度；渲染测试用显式 light/dark 验证用户块、正文、帮助、补全和选择器。

### 语义色修复与验收（2026-10-15）

终端正文为紫色时，旧的 `ensure_readable` 把混色权重用反了：第一次尝试直接得到终端前景，accent、code、warning、error 因而全部变成同一种紫色。原来的对比度测试没有检出这种颜色退化。现在先保留可读的语义色，仅在对比度不足时向终端前景混合；回归同时检查对比度和颜色区分，包括紫色正文配奶油色背景。具体角色用途见下方“主题角色”。

Supervisor 验收：全量串行测试主程序 251 项、实验模块 40 项通过，6 项手动 fixture 忽略；TUI 子集 171 项通过、5 项忽略；格式与构建通过。实际二进制的 8 个受控 PTY 场景通过，覆盖迟到颜色、切色刷新、手动主题、编辑／粘贴、滚动／缩放和退出清理；fixture history 未变。实际 tmux 输出确认 Activity／表头为绿色、inline code 为蓝色、正文继续继承终端前景。`semantic_render_capture` 生成实际 TestBackend cells 的浅／深色 HTML，已人工检查；该 HTML 不替代物理终端测试。日志：`/tmp/saki-color-session-supervisor-tests.log`、`/tmp/saki-color-session-native-report.json`；渲染捕获：`/tmp/saki-tui-semantic-colors.html`。

### 排版与 Markdown

- 普通正文（包括标题、列表、引用）上限 96 terminal cells，左对齐并保持 3-cell 左边距；表格可以使用完整 viewport 宽度。
- `pulldown-cmark` 启用 `ENABLE_TABLES` 与 `ENABLE_TASKLISTS`。表格按内容分配列宽、单元格换行、显式对齐（左／中／右）、表头加粗与弱横线；列不可读时逐字段回退为 `Header: value`，长字段名换行而不裁掉。
- 列表保留有序编号、嵌套与任务标记；换行续行使用 hanging indent（例如 `1. ` 后续行缩进 3 cells，嵌套按层级增加）。引用行前带次级 `│`。
- 标题：H1 粗体加下划线，H2 粗体，H3 粗斜体；围栏代码块显示语言标签；规则线使用主题 rule 色。
- 复制分两条路径：拖选复制渲染后的可见文本；`copy_source`／`copy_code_source` 返回消息原文与原代码块，排版装饰不进入任一路径。分页以固定宽度下的逻辑可视行为准，源偏移保持单调（允许合成行共享偏移）作为 reflow 回退。

### Activity

- 用户消息提交时在 user record 之后创建一个前端 Activity marker；raw record 顺序和索引保持追加式，不改动已有记录，不根据句子或工具成功猜测完成。
- marker 的成员是随后直到最后一个 Reasoning/Tool 记录；其后的 Assistant 记录作为最终回答保持可见。没有推理或工具的简单对话不渲染任何 Activity chrome。
- `TurnCompleted` 时：正在跟随最新、没有选择文字且未手动展开时，过程折叠为一行 `▸ Activity · n tools · n thoughts`；失败或中断时标签带错误色状态，恢复的错误工具数量保留在摘要中。
- 读者手动展开/折叠的 Activity、工具或 Thought 状态不会被自动折叠覆盖；回看、选择、日志视图或失败期间的阅读位置保持不变。`TurnFailed` 不把过程重新打开，Error record 仍单独可见，部分回答保持原文。
- 恢复会话时由 TaskStarted/TaskFinished 重建分组：Completed 折叠，Failed/Interrupted 保持可见并标记中断；没有任务边界的旧历史不创建 Activity。恢复中未结束的 marker 在 replay 边界关闭，重开后的新任务不会并入旧组。会话切换与重开保留手动 Activity 展开状态（按记录下标转移）。
- Activity 摘要每次按成员记录重新计算，工具完成不会留下旧的计数。鼠标点击与键盘 Enter 都能展开 Activity，展开后仍可逐项操作工具参数、原文输出与 `output_path` 日志。

### 分页、选择与隐藏记录

- `App::visible_record_keys()` 是唯一的显示投影：折叠 Activity 的成员与没有成员的 marker 不会成为显示项，分页和 Shift 选择按投影移动，不会卡在隐藏记录上。anchor 仍用 record 下标加源字节偏移，投影变化时归一化到可见项。

### 原生 Termina 主题（2026-10-15，当前实现）

- 输入与 raw mode 由 Termina `PlatformTerminal` 持有，Ratatui + `CrosstermBackend` 继续负责渲染；alternate screen、括号粘贴、鼠标、focus 与 mode2031 在正常退出、启动失败和 panic 时恢复（Termina `set_panic_hook` 在旧 hook 前恢复 termios）。
- OSC/CSI 回复在输入适配器内消化，绝不成为编辑文本；正文继承终端前景，衍生 palette 由采样 fg/bg 计算。查询使用有界窗口合并，运行期 scheme 变化只刷新 OSC 10/11，不重复询问 scheme、不按帧轮询。
跨 viewport 的会话选择使用逻辑行身份，停留边缘时以有界定时步进自动滚动；输入/帮助/状态与日志页内保持屏幕内选择。独立 PTY 已确认停留边缘的持续推进与停止条件；endpoint 与宽字符场景在冻结二进制 `0551d6702151…` 上 5/5 通过，复制意图由 Rust 测试精确比对且未写系统剪贴板。

- 已知边界：Termina 0.3.3 不解析 `rgba:` 颜色回复（回退 native，不另写 raw parser）；EventStream helper 对 poll 错误/EOF 会重试且不主动唤醒，ignored-SIGHUP 等断开场景不声明通过；surface 到的 I/O 错误仍向前端传播。物理 Ghostty 交互、OS 剪贴板与真实模型生成不在本轮自动验证内。

验收证据（supervisor + QA，2026-10-15）：受控 PTY 10 个场景全部通过（含分片迟到颜色、排队 scheme 变化在完整 pair 后 0.252 s 无按键触发、手动不查询、输入/粘贴、滚动/resize、正常清理、panic 清理）；PTY panic 父 fixture 1 项通过（0.03 s）；受测不可变二进制 SHA256 `b535cc9ac0c5174265336ff8b91a2e79ceb3ed975be6e8a42d2e22444bd4d296` 前后一致；fixture history SHA256 未变。完整记录见 `/tmp/saki-tui-native-theme-supervisor-report.md`。

### 真实 tmux 验收（2026-10-14，历史：tmux 元数据版）

Supervisor 在隔离的可弃置 detached tmux 会话中用真实二进制和已完成 demo 会话完成验收，确认的可观察结果：

- 恢复 demo 会话正确显示：请求、一个折叠 Activity、最终回答与结构化 Markdown 表格及块间距（无模型调用、未重执行任何工具）。
- 鼠标点击（SGR）展开 Activity 与单个工具详情有效；键盘 Tab／Shift+Up／Enter 选中并展开 Activity，选中头部显示共同选择标记 `▌`。
- 窗口 100／48／18 列 resize 保留布局，窄表格回退为 `Check:`／`Result:` 等带字段名的记录；18 列阅读状态底栏保留 `0% / 1M`，可选 Latest／Help 按优先级隐藏（键盘路径仍在）。
- dark／light 运行分别输出各自的 ANSI text、secondary text 与用户背景色；这验证的是主题角色选择，不是 Ghostty 主题的物理外观。
- 多行中文／emoji bracketed paste 保留草稿且不自动发送；Ctrl+C 先清空草稿，空输入再按一次退出；F1 帮助与 Esc 关闭有效；退出后 `stty -a` 报告 `icanon` 与 `echo`，raw mode 已恢复。
- 全程未调用真实模型、未执行任何工具、未写系统剪贴板、未触碰用户其他 pane；临时验收 tmux 会话已移除。demo history SHA256 始终为 `927800174eacdbc8fbf4e3518fcb4e23e8e8ffa9fc677d632106b0cfc6fe9878`。

未由本轮覆盖、仍不声称的边界：物理 Ghostty 按键／选择／鼠标／⌘C／⌘V 行为、macOS 系统剪贴板读写、真实流式模型生成、其他终端与平台、输入法候选窗。

### 尚未实现与限制

- 已知边界：Termina 0.3.3 不解析 `rgba:` 颜色回复，此时停在 native fallback，不另写 raw parser；EventStream helper 对 poll 错误/EOF 会重试且不主动唤醒，ignored-SIGHUP 等断开场景不声明通过。主题变化会清空缓存布局并在会话切换时跟随，不重置草稿、选择与阅读锚点。
- 本轮未做物理 Ghostty 交互验证，也不验证系统剪贴板路径；自动化回归仍以 TestBackend 与确定性事件为准。
- 代码语法高亮与消息点击排队还没有实现；日志继续使用原有有界分页与 `output_path` 入口。会话区跨 viewport 选择已实现，日志跨文件页选择不在范围内。
- Activity 只是前端显示投影，不新增模型总结调用，不改变 compact、JSONL 或 Conversation。

## 目标与视觉语言

用户首先看见请求和结果，需要时查看过程与依据。保持单栏、共同正文起点、普通等宽字号、细分隔线和低对比用户矩形。SAKI 的青绿色用于当前活动与可交互位置，正文使用中性色；成功无需铺满绿色。所有层级依靠粗体、空行、缩进和语义标记表达。

默认阅读顺序：

```text
› User request

  ▸ Activity · 3 tool calls

  Outcome in one clear paragraph.

  Relevant details
  • Evidence and limitations when useful.

  Ready
  ──────────────────────────────────────────
  › Ask a follow-up
  12% / 1M                deepseek-v4.1-flash
```

简单对话没有活动内容时省略 Activity。不要为每段正文追加 SAKI 或 Assistant 标签，不强制模型每次生成相同的章节。

## 信息层次与状态

| 内容 | 默认视觉与交互 |
| --- | --- |
| 用户请求 | 主题内低对比矩形，首行 `›`，上下各一行；文字起点与回答相同 |
| 最终回答／当前流式回答 | 最清晰的中性正文，正常字号；重点仅对少量短语加粗 |
| 活动 | 次级色；动作 + 对象；展开查看按真实时间排序的过程 |
| 正在生成的推理 | 当前轮活动内展开，以可读的次级色显示模型实际返回内容 |
| 已完成推理 | 普通阅读中不逐条保留 Thought；在 Activity 内可定位、展开与复制 |
| 需要处理的失败／中断 | 明确文字 + 错误色，折叠后仍有可见状态与入口；局部回复标为未完成 |
| 已恢复的工具错误 | Activity 摘要保留错误数量／状态，详情保留原始失败及随后执行顺序 |
| 元数据 | 次级色；当前 context 百分比／上限、模型和必要的会话信息 |
| 临时操作提示 | 按需显示，不能覆盖错误或正在执行的状态 |

Activity 是显示分组，不新增模型总结调用，也不改变 JSONL、Conversation 或 compact 的语义。完成前逐条流式展示；正常完成且用户跟随最新时可收起过程。用户手动展开、选择文字或回看时保持阅读位置，不能自动折叠正在阅读的内容。

分组依赖用户请求、任务完成／失败事件和实际工具调用。流式正文出现时正常展示；只有后续工具调用确立了它属于中间过程，才可在最终分组中收纳。失败时不得把最后一段不完整正文当成最终结论。恢复会话使用持久任务边界，不从句子内容猜测“已经完成”。

若未实现任务级分组，可先合并相邻工具与推理入口；不要临时用文案规则隐藏 Assistant 消息。

## 排版规则

| 元素 | 规则 |
| --- | --- |
| 正文 | 建议上限 96 terminal cells，左对齐；更窄时使用可用宽度。这个值需要实测，不是 Pi／Codex 的统一默认值 |
| 对齐 | 保留当前 3-cell 正文左边距；极窄宽度减少边距，不左右交替气泡 |
| 间距 | 段落／章节间一空行；列表项和同组工具紧凑；避免容器和子组件同时加间距 |
| 标题 | H1 粗体并可用下划线，H2 粗体，H3 粗斜体；不改变字号，不给整段正文着强调色 |
| 列表 | 续行对齐条目正文；保留编号、嵌套与任务标记 |
| Inline code | 使用主题中的 code 前景，保留原符号，不全局转换成按钮 |
| 代码块 | 语言名与细边界，保持缩进；可使用比正文更宽的空间；软换行只影响显示，复制保持原始代码 |
| 引用 | 单根次级竖线与次级正文，不额外增加完整外框 |
| 表格 | 表头加粗、弱化横线、按内容分配列宽、单元格换行；尊重显式对齐。窄屏转每行一组 `Header: value`，字段全部保留 |
| 文件／命令 | 优先显示工作目录相对路径，完整值可查看／复制；路径和单词按显示 cells 换行，不能破坏 Unicode 字形 |

表格和代码可放宽到当前 viewport，但不扩大普通段落。先支持明确的 Markdown 对齐，不通过猜测任意文本含义自动右对齐或挑选“重要数字”。未闭合的流式代码块应保持代码样式；表格形成足够结构前保留稳定的原文显示，结束后再完成布局。

## 主题角色

以下色值用于手动 `light`/`dark` 与设计预览；`auto` 使用 Termina 原生采样推导 palette，不把这些固定值当作实际终端的可靠自动探测。物理 Ghostty／OS 外观切换仍不在已验证范围内。

语义用法：H1–H3 标题与 `**strong**` 使用 accent 作为显式重点；表格表头与 fallback 字段名同样用 accent，但 inline code 保留 code 蓝／青。工具状态只在标记上着色（成功 accent 绿、运行 warning 琥珀、失败 error 红），命令文本保持 secondary；Activity／Thought 的 disclosure 标记使用 accent，计数与细节保持 secondary，避免整行着色。

| 角色 | Light | Dark | 用途 |
| --- | --- | --- | --- |
| Background | `#f7f8f5` | `#191d1c` | 预览背景；真实终端继续保留主题背景 |
| Text | `#29332f` | `#dce4de` | 正文和关键值（已接入普通正文与输入） |
| Secondary | `#626e67` | `#9aa9a0` | 活动、推理与元数据 |
| Rule | `#c8d2ca` | `#3b4840` | 次级分隔线与表格横线 |
| User surface | `#e8eee7` | `#27392f` | 用户矩形 |
| Accent | `#32634e` | `#8bbda2` | 当前活动、焦点和链接 |
| Warning | `#825e20` | `#d9b674` | 提醒；结合文字 |
| Error | `#a33d3d` | `#e39792` | 需要处理的错误；结合文字 |
| Code | `#2c5d7a` | `#86b8d6` | inline code 与代码块，蓝／青色技术语义；语言标签使用 rule 色 |

用户块的前景与背景作为一对选择，不能只换底色。装饰线可以更弱，Secondary 不能依赖 dim 后变得不可读。代码与 diff 的额外颜色只承担语法和增删语义。

## 四工具的显示

默认行围绕真实参数生成，例如 `Read src/agent/mod.rs`、`Write docs/design/example.md`、`Edit src/frontend/tui/view.rs`、`Run cargo test`。工具结果完成后展示明确状态；完整参数、原文输出和日志入口在详情中。复杂 bash 直接显示真实命令，不先造一个不可靠的自然语言标题。不得从路径名猜测运行成功或从工具成功推导任务正确。

折叠只省显示面积。工具层已有输出上限和 `output_path` 的规则继续生效；界面必须区分“有更多可展开内容”与“消息只保存了预览，需要打开日志”。未知／非零退出码按实际结果表示，不自行把搜索无匹配等同于执行故障。

## 输入区与状态栏

执行状态位于输入区上方，避免在历史中反复追加 Working／Ready；帮助和会话操作沿用现有按需入口。输入区只显示草稿／附件和轻量占位提示。下方左侧保留用户要求的 `12% / 1M`，右侧模型名，不重新堆满输入／输出／cache／费用计数，也不放无信息量的 SAKI 标识。

窄屏先省略可选目录／分支与额外会话文案，再缩短模型显示；完整名称可在详情查看。context 百分比和必要的错误状态优先保留。必须换行时使用明确的第二行，不能相互覆盖。真实 Agent 状态决定文案；尚未实现主动取消时不显示可取消提示。

## 模型输出与 renderer 的边界

模型需要尽早给出结论，工具过程中只说明有用的发现与下一步，按任务需要使用少量标题、列表或表格。renderer 负责忠实组织这些结构，不重新生成更漂亮的结论、不删去反例，也不声称校验了回复中的数字。输出约定应保持简短，不能拿一个长风格 prompt 代替排版。

本轮仅提出约定。以后实施时先确认已有 system instructions，避免新增另一条互相冲突的输出规范。

## 实施顺序与验收

1. 主题角色与成对颜色；修复 Markdown 表格、列表续行和标题层次。
2. 正文宽度和块间距；保留原文字节与渲染行映射、复制和滚动锚点。
3. 以真实任务边界投影 Activity，收纳完成推理与工具；保持错误、恢复和手动展开语义。
4. 在真实 Ghostty／tmux 验收长工具链、流式内容、resize 与粘贴；按观测调整密度和宽度。

改动应集中在 `frontend/tui/`，复用现有 Markdown、记录状态与交互路径。Agent、SessionManager、模型协议和 JSONL 不因这次视觉设计引入新的依赖。如果持久边界缺失，应明确补充最小公共事件，不让核心了解布局和颜色。

| 场景 | 可观察的验收结果 |
| --- | --- |
| 多轮工具调用后完成 | 最新答案清楚可见；一次展开能找到完整过程；没有一串独立 Thought 占位 |
| 失败／响应中断 | 默认可见失败状态；未完成正文不标为成功结果；能定位原始依据 |
| 推理流式到结束 | 灰色真实内容可读；完成后可收起并重新打开；手动阅读不被抢走 |
| 宽／窄终端与 CJK | 正文不横跨超宽屏；列表和表格保持结构；字形、草稿和阅读锚点完整 |
| 代码、命令与表格复制 | 原文复制不混入边框、显示换行、标题或省略符 |
| 用户折叠／展开过程后有新消息 | 选择与阅读位置稳定；跟随最新与回看行为可预测 |

HTML 示意只能验收设计的浏览器交互、颜色与响应式布局，不能证明 Ratatui、ANSI、tmux 或系统剪贴板行为通过。

2026-10-14 已检查示意的三个状态，在 Light／Dark、320／736／1024 像素内容宽度下共 18 个组合无横向溢出或脚本异常，同宽度状态保持相同初始高度。已验证 Activity 展开、键盘打开 Thought、场景切换保留草稿／展开状态、按需帮助，以及外观／阅读宽度／间距设置生效；人工查看了宽屏深色与窄屏浅色截图。78 个相关文档本地链接检查通过。以上是 HTML 示意阶段的记录（当时本轮尚未修改 Rust 代码，因此没有重复运行 Agent 测试）；实际 Rust 实现与真实 tmux 验收结果见上文“实验实现”与“真实 tmux 验收”。
