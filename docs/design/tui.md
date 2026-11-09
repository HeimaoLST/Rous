# Rous：终端产品与实现

下一版显示层次和视觉规范见 [TUI style](tui-style.md)，Pi／Codex 的对应源码分析见 [Terminal presentation](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-presentation.md)。TUI style 实验已于 2026-10-14 实现（主题、正文宽度、Markdown 表格、Activity 分组），本节继续描述已实现行为。

更新日期：2026-11-09。原始首版记录日期为 2026-10-23。首版已采用“工作记录”布局、Ratatui 原生组件和独立终端前端。Pi 负责具体实现，Codex 负责分阶段审查与独立验收。此前的布局和组件研究见 [Terminal UI 研究](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-ui.md)，HTML 对比稿仍保留为设计材料。

## 目标与当前能力

让用户在长任务中容易理解正在发生什么、找到结论、查看执行依据并继续输入。SAKI 的识别度来自长期协作中的一致行为和清晰的信息组织，界面为这种关系提供承载。

当前 Rous 是独立 Git 仓库中的终端客户端。Saki 是独立进程，拥有唯一模型与工具执行循环。协议权威见 [Saki Runtime API](https://github.com/HeimaoLST/Saki/blob/dev/docs/design/runtime-api.md)。Rous 保留多行编辑、图片输入、主题、工作记录、Markdown、工具展开、复制、草稿、会话选择与取消交互，不在客户端重建执行状态机。

### 2026-11-09 独立产品实现事实

- `src/app.rs` 只选择前端和外部 Runtime，使用 `SAKI_RUNTIME_BIN` 或 PATH 中的 `Saki`。`src/client/mod.rs` 持续读取 stdio，以请求 ID 路由 response，按会话、任务和消息 ID 投递 event。每个 Rous 拥有一个子进程，连接断开后的 EOF/shutdown 收尾仍由 Saki 执行。
- `src/session/terminal.rs` 将公开 RPC 适配为 UI packet。Send 的 Accepted 先于该任务的 UI 事件；查看其他会话不取消当前任务。metadata invalidation 触发公开查询，标题由 Saki 生成。`history_required` 在 Finished 前通过公开 snapshot 修复显示。
- `src/client/history.rs` 只保留显示记录与状态，不包含 journal 字节偏移、写入修复、Conversation、provider 或存储 replay。运行中重新打开会话时，`before_task_id` 返回的前缀状态按前缀的语义记录计算。
- 客户端事件积压受 8 MiB 字节预算约束，不因第 129 个小事件断连。写入失败或超时会关闭可能损坏的 stdin，并返回 `delivery_unknown`；仅响应超时保留完整连接，不自动重发。启动预算为 45 秒，启动失败回收子进程与 stdout/stderr reader。
- `src/session/channel.rs` 的局部展示关闭信号独立于执行核心。异步入队超时、超大投影或关键回执无法入队后，展示永久关闭、排队投影作废，后续发送拒绝。manager 停止接纳并正常 drain 已接受的 Runtime 工作；保留 receiver 不会掩盖失效，也不会自动恢复不完整显示。
- 旧 UI 测试使用公开 DTO fixture；两个图片路径分类测试在 `tests/input/mod.rs` 承接。原四个混合 session 场景由 `client::tests::real_runtime_contract` 通过真正的 Saki 子进程覆盖。旧手工构造 SessionStore 的 demo fixture 已退役，普通渲染与输入测试保留。
- 迁移验证使用本地受控 provider、真实 stdio 子进程及 PTY。历史物理终端、剪贴板、模型或真人报告不能直接算作本次验证；原 MVP 未完成状态不变。

## 首版使用与模块边界

```sh
# stdin 和 stdout 都是终端时默认进入 TUI
SAKI_RUNTIME_BIN=/absolute/path/to/Saki cargo run --bin rous

# 使用同一外部服务的 console 客户端
SAKI_UI=console SAKI_RUNTIME_BIN=/absolute/path/to/Saki cargo run --bin rous
```

不设置 `SAKI_UI` 时，管道和重定向自动使用 console。显式 `SAKI_UI=tui` 要求 stdin/stdout 都是终端，否则返回错误。

| 模块 | 职责 |
| --- | --- |
| [main.rs](../../src/main.rs) | 模块声明与启动 `app::run` |
| [app.rs](../../src/app.rs) | 选择前端、启动外部 Runtime 并等待收尾 |
| [client/mod.rs](../../src/client/mod.rs) | 子进程、公开 stdio DTO、请求 ID 路由与字节预算 |
| [input.rs](../../src/input.rs) | 前端文字、图片、附件快照与输入分类 |
| [session/terminal.rs](../../src/session/terminal.rs) | 公开协议到 UI packet 的适配 |
| [frontend/console.rs](../../src/frontend/console.rs) | 永久保留的纯文本前端，EOF 后仍消费当前结果 |
| [frontend/tui/](../../src/frontend/tui/) | 输入、展示状态、Markdown、记录排版、日志浏览和终端恢复 |

依赖方向为前端 → terminal adapter → 公开协议客户端 → Saki 进程。Rous 不持有 Agent、Conversation、ToolRegistry 或模型客户端。Runtime 配置和存储属于 Saki，Rous 保留现有配置名与数据路径。下面的交互规范仍适用；带旧日期、旧 commit 和验收数字的段落是历史记录。未随产品导出的研究和核心源码链接固定到原始候选。

| 操作 | 行为 |
| --- | --- |
| F1 / 空草稿时 ? / 点击帮助 | 在输入框上方打开帮助；Esc、F1、Ctrl+C 或点击关闭帮助收起，不改变草稿、焦点或阅读位置；↑↓、PgUp/PgDn、Home/End 与帮助区滚轮浏览说明；帮助列出会话/输入历史搜索控制与接受/关闭语义，并区分 Esc 先关局部视图再中断任务、Ctrl+C 不取消任务 |
| 点击回到最新 | 回看或日志视图中恢复跟随最新输出，保留草稿；有新输出提示同时清除 |
| Enter / Alt+Enter / Ctrl+J | 输入区发送／换行；兼容增强键盘协议下的 Shift+Enter；多行粘贴保留换行，不自动发送 |
| Ctrl+V / 粘贴完整图片路径 | 输入区添加图片附件；剪贴板没有图片时粘贴文字；允许文字与多图或仅图片发送 |
| Backspace（文字为空且有附件） | 删除最后一张图片 |
| 输入区快捷编辑 | Ctrl+B／F 左右字符；Ctrl／Alt+←／→、Alt+B／F 按 Unicode 词界移动；Ctrl+A／E 到逻辑行首尾（在边界可跨到相邻行）；Home／End 留在当前行；Ctrl+W、Alt／Ctrl+Backspace 后删词；Alt+D、Alt／Ctrl+Delete 前删词；Ctrl+U／K 删除到逻辑行首尾（在边界可删除相邻换行）；Ctrl+H／D 删除字符；Ctrl+J 换行；按词操作对齐 grapheme，组合字符与 emoji 不被拆开 |
| 输入区恢复删除 | Ctrl+Y 插入最近一次词／行删除；单条 kill buffer，不是系统剪贴板或通用 undo；跨发送、清空和历史召回保留，普通字符删除不覆盖 |
| 输入区撤销／重做 | Alt+U 撤销、Alt+R 重做（不改 Ctrl+Z 作业控制）；仅输入焦点，局部模态优先。编辑器持有有界历史（最多 100 条，且全部 undo＋redo 保留文本合计 ≤ 8 MiB）：连续单字符输入合并为一步，移动光标／设置光标偏移／粘贴／非输入编辑断开分组；粘贴、kill、yank、图片增删各为一步；无操作不丢弃 redo，撤销后新编辑丢弃 redo。Ctrl+C 清空可撤销且仍进入输入历史；成功发送、会话切换、已执行命令重置撤销历史（不能撤销已执行任务／命令文本）；输入历史召回是可按撤的整草稿替换；kill buffer 不随撤销回退。撤销／重做恢复光标的首选显示列（普通编辑仍重置）；单次编辑的前状态若自身超过字节预算则重置并跳过历史；若撤销／重做时当前草稿无法存入预算，则保持草稿不变且操作不可用 |
| 输入区视觉移动 | ↑／↓、Ctrl+P／N 按实际输入区宽度在软换行之间移动，连续垂直移动保留显示列；满足历史条件时改为召回输入历史 |
| 输入历史 | 发送与 Ctrl+C 清空的完整草稿（文字及附件）进入当前进程历史，相邻重复去重；不从 conversation 重建、不受 compact 影响；空输入可召回，非空输入仅在等于最近召回草稿且光标位于全文头／尾时继续翻历史；改动文字或附件后继续用 ↑／↓ 移动光标；向下越过最新条目恢复空输入 |
| 输入历史搜索 | Ctrl+R 打开输入框上方的本地搜索（空查询，最新匹配在前）；查询大小写不敏感，Ctrl+R／↑ 更旧、↓ 更新；Enter 把选中条目送入编辑器供编辑、不发送；Esc／Ctrl+C 取消并完整保留原草稿、光标、kill buffer 与附件；搜索期间真实编辑器不变，文本粘贴只改 query（多行折叠），图片粘贴提示先关闭搜索；F1 关闭搜索并打开帮助，Ctrl+S／会话标题点击先关闭搜索再打开 picker |
| Tab（路径补全） | 输入焦点对光标所在路径分段做文本补全，基于当前会话 working_directory；支持 ./ ../、普通相对名、绝对路径与 ~/（仅 `~user` 不支持）。命令补全优先；有候选时单候选直接补全，多候选弹出紧凑列表：目录在前，按 Unicode 小写名再原文排序，最多保留 50 条并在截断时显示 “N more — type to narrow”。Up/Down 选择，Enter/Tab 确认且永不发送，Esc 关闭且不改文本；目录补全追加 / 并关闭，再按 Tab 列出子项。只替换光标所在分段，已有分隔符与闭引号保留；匹配只用光标前的文本，光标后的分段不被复制；目录仅在尚无分隔符时追加 /，若后面还有 /child 则只列目录候选；保留引号、反斜杠转义、空格/Unicode；一次补全为一步 undo，图片与 kill buffer 保留，纯无操作不丢弃 redo。无候选或空输入时 Tab 保持原焦点切换。匹配大小写不敏感，不以 . 开头时不列点文件；仅按需读取目录元数据，不读文件内容、不递归、不建索引。这是路径文本补全，不是 shell 展开（无变量/通配/命令替换） |
| Ctrl+G（外部编辑器） | 输入焦点把当前草稿写入临时 UTF-8 文件，用第一个非空 `VISUAL` 否则 `EDITOR` 的配置启动编辑器；命令经 shlex 解析，带引号的可执行路径与参数可用，临时文件作为最后一个参数直接传给进程。不使用 shell、不做变量/通配展开、不猜测 vi 回退。编辑器成功退出 0 后把合法 UTF-8 文本读回草稿、不发送；改变文本是一次 undo（光标到末尾），图片与 kill buffer 保留，完全相同的文本不写历史也不移动光标。缺失配置、命令格式错误、启动失败、非零退出、读取失败或非 UTF-8 只给英文提示，原草稿/光标/图片/历史不变，临时文件总在返回前清理。运行期间保持同一个 `PlatformTerminal`：先确定性停止并 join 输入读取线程、恢复输出模式与捕获的 cooked termios，再启动编辑器；返回后在同一句柄重新进入 raw/输出模式、重启读取并强制整屏重绘。`Ctrl+C` 在编辑器内不会杀死父进程（等待期间安装被捕获的 SIGINT 处理，exec 在子进程恢复默认）。帮助/历史搜索/转写搜索/日志/会话选择器优先；路径弹窗先关闭；不支持编辑器内异步排队输入或持久化。 |
| 代码围栏高亮 | 仅对带可识别语言标签的 fenced code block 使用 syntect 解析（`default-syntaxes` + `regex-fancy`，进程内静态 `SyntaxSet` 只加载一次）；按整块状态解析，支持跨行字符串/注释。角色仅缓存字节范围（关键词/字符串/数字/注释/其余），按现有 `Theme` 语义前景与修饰着色：关键词 accent+bold、字符串 code、数字 warning、注释 secondary+italic、其余 text；不导入外部主题、不新增调色板。未知语言、无标签与缩进代码保持原有单色（code）与原始标签；原始记录、复制、换行与 source/advance 完全不变，复用现有 Markdown offset_iter 与布局缓存。解析结果缓存在本地有界缓存（≤64 项、保留代码总计 ≤2MiB，键为语言 token+完整代码），宽度或主题变化只重排不重解析，内容/语言变化会失效。语言别名通过 syntect 的扩展名/名称查找（如 rust/rs、py），未收录语言走单色回退；不提供全部语言保证、行号、树形控件或用户设置。 |
| 点击 Thought / 记录区 Enter | 展开／收起工具或推理；推理期间浅灰展开，结束后自动折叠，回看或手动展开时保留状态；完整原文继续可复制 |
| Tab / Esc | Tab 切换输入和记录焦点；Esc 依次关闭输入历史搜索、帮助、日志视图或清除真实文本选择，空闲时退出记录焦点，任务运行时发送取消（保留草稿） |
| 鼠标滚轮 | 指针在记录／日志区时上下滚动，每次 3 行；不改变焦点和输入草稿 |
| PgUp / PgDn | 按当前视口高度翻页；输入与记录焦点均可使用 |
| Alt+Up / Alt+Down | 任意焦点逐行滚动，输入区普通方向键继续编辑草稿 |
| Ctrl+Shift+Up / Ctrl+Shift+Down | 跳转到上一条／下一条真实用户消息边界；仅在普通记录／输入界面生效，帮助、输入历史搜索、日志与会话选择器优先。长消息内部先回到该消息起点，已在首行则跳到上一条；跳跃停止 follow 并保留草稿、光标、附件与焦点；无更早／更晚消息时给出英文提示 |
| F3（记录搜索） | 在普通输入／记录界面搜索当前已加载记录的**渲染文本**；大小写不敏感，显示匹配总数与当前位置，无匹配／空查询有明确提示。Enter／F3／Down 下一个，Shift+F3／Up 上一个（循环）；Esc／Ctrl+C 关闭并恢复原 follow、源锚点／视觉锚点与焦点，草稿、光标、附件与 undo 不受影响。查询只接收文字粘贴（换行折叠为空格），图片粘贴提示先关闭搜索。折叠 Activity／工具／推理的隐藏正文不索引，展开后才可搜索；不读取磁盘日志、不搜索原始 Markdown 语法。匹配只落在同一渲染行内，跨软换行的匹配不报告（见限制）。搜索期间命令补全弹窗与输入框硬件光标暂停，草稿不会被 Tab／方向键改写；输入框光标由搜索条自身的查询光标替代（窄宽度保留查询尾与光标）。帮助、输入历史搜索、日志与会话选择器优先；F1 或会话切换先关闭搜索 |
| 记录区 Up / Down | 逐行滚动长消息；Shift+Up / Shift+Down 选择记录 |
| 记录区 Home / End；Ctrl+Home / Ctrl+End | 跳到对话开头／最新输出；Ctrl 组合键在任意焦点可用，日志视图会先关闭 |
| 点击 Thought / 记录区 Enter | 展开／收起选中的工具 |
| 记录区 y / Y | 复制消息原文或工具命令／选中消息的代码块正文或工具输出 |
| Ghostty Shift+拖选，然后 ⌘C | 使用终端原生文本选择复制屏幕内容；⌘V 由终端发起粘贴 |
| 普通鼠标拖选 | 高亮并跨 viewport 选择记录区可见文字，松开后一次性复制到系统剪贴板；停在上下边缘时约 60 ms 步进自动滚动；无需 Shift |
| 工具记录 l | 打开 `output_path` 日志；Up/Down 页内滚动，PgUp/PgDn 浏览页面 |
| Ctrl+C | 优先关闭帮助，再关闭日志局部视图；否则有草稿时保存到输入历史并清空（忙时也只清草稿，不请求退出）；空输入时空闲退出、运行中等待当前轮结束后退出并如实提示，不宣称已取消任务 |
| 输入区 Ctrl+D | 非空输入向前删除一个字符；空输入复用 Ctrl+C 的退出／等待路径 |

运行中可以继续编辑草稿，Enter 保留草稿并提示稍后发送；没有隐式排队。compact 只改变模型上下文，界面追加压缩记录，已展示内容与阅读位置继续保留。

日志每次最多读取 16 KiB／300 行，超长行有截断标记，磁盘原文不改动；翻页位置按磁盘原始字节计算。复制正文不会混入角色、状态或边框装饰，代码正文保留末尾换行。界面捕获鼠标以接收滚轮；帮助打开时只滚动帮助，平时在记录／日志区域处理竖向滚动；日志滚轮在当前文件页内滚动，跨文件页继续使用 PgUp/PgDn。底栏帮助与回到最新支持点击；暂不实现消息点击选择与滚动条。系统剪贴板失败会显示提示。

用户当前终端为 Ghostty，tmux 开启 mouse。鼠标报告开启时普通拖动会转发给应用，因此 SAKI 自己处理普通拖选、高亮与松开复制。记录区选择使用逻辑行身份（record 与局部可视行）加终端列，viewport 滚动后端点仍有效；停在上下边缘时按约 60 ms 的有界步进自动滚动，松手、取消、失焦或视图／会话变化停止，文档边界不空转。拖选复制渲染后的可见换行、缩进与字形并去掉行末填充；y／Y 仍复制消息原文、命令或代码块原文。中文宽字符、emoji、组合字符作为完整字形复制。选区存在时 Ctrl+C 优先复制，不清空草稿或退出。滚动、键盘编辑、resize 或选中行内容变化清除选区；单纯更新其他位置的模型／上下文元数据不会清除。输入、帮助、状态栏与日志页内保持屏幕内选择；日志跨文件页选择与选区替换编辑未实现。

Ghostty 也允许 Shift 绕过鼠标报告来进行原生选择，见 [Shift-Escape 文档](https://ghostty.org/docs/vt/csi/xtshiftescape) 和 [mouse-shift-capture 配置](https://ghostty.org/docs/config/reference#mouse-shift-capture)。这条路径仍可配合 ⌘C；精确原文与代码继续使用 y／Y。⌘V 的终端粘贴与 Ctrl+V 的应用剪贴板读取都会回到输入焦点，保留多行草稿且不自动发送。没有修改用户的 Ghostty 或 tmux 配置。其他终端可能使用不同原生选择修饰键。

## 消息与底栏的视觉调整

用户消息使用覆盖记录区域宽度的低对比主题底色，搭配主题正文色。上下各留一行，正常宽度保留右侧内边距；多行输入与自动换行处于同一个背景块。背景覆盖空余 cells，选择标记落在正文行。浅深主题默认由 `SAKI_THEME=auto` 通过 Termina 原生输入读取器采样 OSC 10/11 并订阅 mode2031/CSI ?997 跟随；正文继承终端前景，衍生色从采样 fg/bg 推导，未知或不可读时回退 native。

Assistant 消息和 Markdown 段落、标题、列表、代码块之间保留空行；列表项、代码正文和连续工具记录保持紧凑。装饰与留白只影响展示；阅读滚动同时维护原文字节 anchor（reflow 回退）与固定宽度下的逻辑可视行 anchor，PageUp 可以跨越背景内边距与记录边界。拖选复制渲染文本，y／Y 复制原文；两者都不混入角色、状态或边框装饰。

用户实测后取消 Assistant 的行内 `SAKI` 标签和续行额外缩进。首行与所有续行共用 3 cell 左边距，对齐 User 箭头后的正文列，宽度不足 4 时去掉边距。用户背景区分双方，页脚保留 SAKI 标识；不追加每条回复的角色标题。列表和代码自己的缩进继续保留；工具摘要使用细竖线与弱化颜色，正文共用同一左边距。工具摘要压为一行，超出时显示省略号；展开参数和原文复制仍使用完整数据。

### 2026-10-14 TUI style 实验实现

已按 [TUI style](tui-style.md) 完成下列前端调整，未改 Agent、协议、SessionManager、JSONL 或 console：

- `theme.rs` 引入成对 light/dark 角色与 native fallback，默认 `SAKI_THEME=auto`：输入改由 Termina `EventReader`/`EventStream` 统一读取，启动时发送 OSC 10/11、启用 mode2031 并用 CSI ?996 建立初始 scheme baseline；运行期 CSI ?997 变化只刷新 OSC 10/11，不重复询问 scheme。正文始终 `Color::Reset` 继承终端前景；user surface、secondary、accent、code、warning、error 从采样 fg/bg 推导并保持全部 text-bearing 角色 ≥4.5 对比度，未知或不可读时回退 native。手动 `SAKI_THEME=light|dark` 固定且不发送查询、不启用订阅；未知值静默回 auto。`App.theme` 是 view、help、命令补全、会话选择器、日志、输入、状态栏与选择高亮的唯一渲染调色板。
- 普通正文宽度上限 96 cells；表格可使用完整 viewport。`pulldown-cmark` 启用表格与任务列表：表格列宽分配、单元格换行、显式对齐、表头加粗与弱横线；窄屏回退为 `Header: value` 并保留全部字段。标题层次、编号／嵌套／任务列表的 hanging indent、引用、规则线与围栏语言标签已接入。
- 任务级 Activity 在用户消息后创建，按 TurnCompleted/TurnFailed 与持久任务边界折叠：完成且跟随最新时显示一行 Activity，手动展开、回看、选择或失败时保留过程与阅读位置；简单对话不出现空 Activity。错误恢复数量、失败与中断状态在折叠后仍可见。
- 分页与记录选择基于显示投影，隐藏／空的 Activity marker 不会成为可导航项；原文与代码复制路径不变。
- 验证（2026-10-15 原生主题 + 语义色）：`cargo test --bin Saki tui:: -- --test-threads=1` 当前 185 项通过、5 项手动/TTY fixture 忽略；组合全量串行套件由 supervisor 验证：主程序 265 项、实验模块 40 项通过；`cargo fmt --all -- --check`、`cargo build` 通过（生产警告与基线相同 4 项）。跨 viewport 拖选已在冻结二进制 SHA `0551d6702151…` 上验收通过：root 组合串行与 fmt 通过（计数见上）；QA strict run-native-06 在 23 秒内 5/5 PTY 场景通过（报告 `/tmp/saki-selection-autoscroll-qa/report.json`，history 与二进制前后不变），覆盖停留上边缘无输入持续推进（marker 119→90）、interior／focus／cancel 停止、endpoint 与宽字符 emulator 场景；复制意图在 Rust 测试中精确比对，未写系统剪贴板。日志仍保持 16 KiB／300 行的有界分页，跨文件页选择未实现。主题回归要求 light/dark 与采样推导 palette 在所有实际表面上 text-bearing 角色对比度 ≥ 4.5。手动渲染快照、语义色实际渲染 capture（`/tmp/saki-tui-semantic-colors.html`，包含 purple-on-cream auto palette）与 TTY panic 生命周期 fixture 均为 ignored 手动运行。原生输入适配回归覆盖 Termina 分片 OSC/CSI 与按键交错、括号粘贴内伪转义保持原文、鼠标滚动/拖选与编辑绑定。未验证边界：物理 Ghostty 交互、macOS 系统剪贴板、真实模型生成、Termina 0.3.3 不解析 `rgba:` 回复（回退 native）、以及 EventStream helper 对 poll 错误/EOF 的重试边界（ignored-SIGHUP 等断开场景不声称通过）。

2026-10-10 已实现“工作记录”交互调整：

- 底栏的初版简化方案随后按 2026-10-11 用户反馈修订为模型／上下文状态栏，见下节。完整快捷键仍按需展开；最近响应的输入／输出 usage 在帮助中查看。
- 执行状态独立于提示：工具开始后显示实际工具摘要，运行中回看仍显示执行中；等待退出显示真实等待状态。复制成功／失败单独显示三秒，由到期定时器移除，不覆盖执行状态。
- 新回复、工具完成、压缩和错误在回看／日志中标记“有新输出”；usage 和用户自己的提交不标记。新结果不移动阅读锚点；点击回到最新、Ctrl+End 或向下滚动到底部后清除提示。日志原先处于跟随模式时，关闭日志也恢复跟随并清除提示。
- 帮助位于输入框上方，按会话、输入、记录和帮助分组，随终端宽度换行；高窗口最多占十四行，小窗口保留输入和底栏并允许滚动。F1 可从输入、记录或日志打开；`?` 只在输入焦点且文字和附件都为空时打开，已有草稿时保持文字语义。
- 帮助打开时导航键只浏览帮助，Enter 不提交，Ctrl+C 优先关闭帮助而不清草稿／退出。bracketed paste 收起帮助后按已有文字／图片路径逻辑处理。

界面职责聚合于 `src/frontend/tui/`：`help.rs` 管理帮助内容、换行与滚动，`view.rs` 负责布局和鼠标命中区域，`state.rs` 保存展示状态，事件循环处理按键、点击和提示到期。10 月 10 日调整没有改动 Agent 或 Conversation；10 月 11 日为状态栏接入下述通用事件，当时未引入 command 系统、取消或消息排队。

### 模型与上下文状态栏、英文界面

2026-10-11 用户明确要求恢复状态栏的信息作用，移除无实际用途的 SAKI 标识，并统一使用英文产品文案。根据用户提供的 Codex／Pi 截图，活动信息放在输入框上方，会话元数据放在下方：

```text
Working · bash cargo test
────────────────────────────────────────────────────────────────────
Input text
12% / 1M                               deepseek-v4.1-flash      Help
```

这是格式示例，容量来自实际选定模型的已解析配置，并非固定 1M。2026-10-12 按用户反馈进一步精简：底栏左侧只显示取整的上下文使用百分比与最大容量，移除 `ctx`、`~`、具体 token 用量和目录；右侧显示模型名与操作入口。工作目录和详细上下文数据在 Help 中查看。活动区使用 `Ready`、`Draft`、`Working`、`Reading`、`Records`、`Log`、`Waiting to exit`；运行时按剩余空间显示工具摘要，摘要放不下时仅显示工具名。回看新结果显示 `New output`，底栏保留 `Latest` 与 `Help` 点击入口。

窗口缩窄时先省略模型的 provider 前缀，再隐藏模型名；context 百分比／容量先于可选的 Latest／Help 按钮预留空间，放不下时先隐藏按钮（键盘路径保留），容量信息实在放不下才隐藏。高度不超过四行时活动状态折叠到底栏；窗口过小时操作入口仍可用原有键盘操作。完整工作目录、模型 ID、context token 数、窗口容量和有效 input limit 在帮助的 `Session` 中可查看。k／M 使用十进制单位。

显示精简不改变计数方式：百分比仍来自 Conversation 对当前活动请求的估算，包含 system、历史消息与工具定义，不包含尚未提交的草稿。它复用自动压缩使用的 `estimated_context_tokens`，由上游 response usage 校准，不另加 tokenizer 或 TUI 自己的计数；估算属性在 Help 的详细信息中说明。容量与百分比使用总 context window；实际自动压缩的 input limit 还扣除 output reserve，并受模型独立 input limit 限制。缺失信息显示 `?`／`unknown`，不会伪装为零。

Agent 通过 `ContextUpdated` 在 worker 启动、用户提交、响应 usage、工具结果和 compact 后推送当前快照。该事件不会追加界面记录、触发新输出提示或移动阅读锚点；console 忽略这一展示事件。组装层将已有的实际工作目录单独传给 TUI，Agent 不承担路径展示职责。TUI 不读取环境变量、查询 model catalog 或访问 Conversation。遥测估算失败时显示未知，不中断业务循环。

界面、帮助、图片附件标签、剪贴板／图片错误和运行提示统一为英文。默认 system prompt 和遗留 calculator 的工具说明也改为英文；默认回复使用英文，用户明确要求其他语言时允许切换。用户原文、文件名和工具输出保持原样，中文／emoji 编辑与复制继续支持。历史设计说明里的中文标签代表当时方案。

## 实际验收
2026-10-23 Runtime 终端消费者（#109）：TUI 与 console 改为消费 Runtime 的有界 `Sender<SessionRequest>`/`SessionEventReceiver`，主机为每条请求分配单调 `request_id`，使用 `try_send`，满队列不阻塞显示并保留草稿。发送在 `Accepted`（完整 session/task/message association）前不写 User/Activity/running，接受后才提交；失败保留文字/图片/光标。每个会话维护独立投影（当前与缓存 App/Editor、observed association、finished/canonical task）；Agent 事件按完整 association 路由到当前或缓存视图，忽略 Agent Turn* 重复终态，只由 `Finished` 终结一次；迟到的旧 task Finished 不能终结新 task。`HistoryRestored` 只修复关联视图、从不切换选择，console 以 append-only 方式补打该任务 canonical 正文但不重放终态，随后唯一的 `Finished` 展示错误/persistence/history_required；`TitleUpdated` 不切视图或清草稿。U03 允许 A 运行时查看/新建/切换 B 而不取消 A，缓存 A 的 live 流与编辑器；取消只针对当前选中任务的精确 session+task，取消确认不是终态。console 增加当前活动任务 Ctrl-C 取消。Ctrl+G 外部编辑器改为后台子进程期间继续消费 Runtime 事件，完成后恢复终端；输出关闭/退出请求会等待编辑器子进程结束再 restore。已知限制：不提供后台/离屏取消 UI；text_selection 保持既有切换清空语义、不跨会话持久化；运行时错误码/原因由 Runtime 定义。宿主 main 在 manager／actor／operations／forwarders drain 之后处理退出，idle SIGINT 无需 stdin EOF；现有 TUI loop 与会话 projection 语义不变。
2026-10-21 代码围栏语法高亮（#30）：仅识别语言标签的 fenced code 使用 syntect（静态 SyntaxSet，default-syntaxes+regex-fancy）按整块状态解析，跨行字符串与注释保持状态；角色范围（关键词、字符串、数字、注释、其余）缓存后按当前 Theme 着色，不引入外部主题。未知/无标签/缩进代码保持原有单色与原始标签，记录原文、复制、换行与 source/advance 不变，现有 Markdown 偏移与布局缓存仍是唯一 authority。缓存有界：≤64 项、保留代码总计 ≤2MiB，键含语言 token 与完整代码；宽度/主题重排命中缓存，内容或语言变化失效。Debug 构建实测（Apple Silicon，本机）：首次 1.9KB Rust 块含 SyntaxSet 初始化约 149ms（Python 2.4KB ≈133ms，JSON 0.85KB ≈12ms），命中缓存重复解析约 1–5µs；首次 40 列布局约 32–44ms，随后同块 40/120 列重排约 0.25–0.7ms。大块/长行探针：100 行/5KB ≈0.20s、1000 行/53KB ≈0.84s；单行 8KB ≈0.89s、32KB ≈1.06s；内容变化后的 15KB/400 行块每次解析 ≈0.17s。基于这些实测，超过 16KiB 的代码块或任一行超过 4KiB 时不做语法解析、保持单色（文本/锚点不变），避免 UI 线程出现接近秒级停顿；这是有界回退，不是异步管线。15KB 级受支持块的首次布局约 223ms（含 380 行渲染），命中缓存后的 40/120 列重排约 4.3ms（按 chunk/run 有序指针扫描，避免逐窗口全量 find）。known/unknown/derived 调色板均复用现有 `Theme`/contrast 测试的颜色角色。已知限制：只覆盖 syntect default-syntaxes 收录/可别名解析的语言，未知或实测超阈值回退单色；不提供行号、编辑器、树形控件或用户配色；不做异步/流式增量解析。验证含已知多角色/跨行状态/相邻块重置、未知/无标签/缩进回退、窄宽/嵌套引用 source/锚点/复制、搜索与选择标记保留语法样式、light/dark/derived 调色板与缓存复用/失效；真实 PTY 见 `/tmp/saki-tui-30-evidence`。

2026-10-21 输入区外部编辑器（#29）：Ctrl+G 在普通输入焦点按 `VISUAL`→`EDITOR` 顺序（第一个非空）解析命令，shlex 处理带引号的可执行路径与参数，临时文件作为末参数直接 spawn，无 `sh -c`、无展开、无回退。成功退出 0 后校验 UTF-8 并作为一次 undo 加载（光标末尾），图片与 kill buffer 保留；完全相同文本不写历史、不移动光标。任何失败只通知并保持原草稿/光标/图片/历史，临时文件清理。终端复用同一个 `PlatformTerminal`：确定性 join 输入读取线程后 restore 输出与 cooked termios，编辑器退出后重新 raw+输出+读取并强制重绘，避免新建终端造成 raw 永久残留；等待编辑器时捕获 SIGINT 防父进程被 Ctrl+C 杀死。实现含可测试的 shlex/临时文件/加载边界、一次 undo/no-op/redo、失败保持、覆盖模态优先级，以及 ignores PTY 读取取消+暂停/恢复 termios 与真实编辑器 PTY。编辑器运行期间暂停输入读取与绘制，但继续消费 Runtime 的有界显示事件并更新会话投影，完成后恢复同一终端与绘制；不支持编辑器内异步输入或 shell 语法。

2026-10-20 输入区路径补全（#28）：Tab 在输入焦点解析光标所在路径分段并列出 working_directory 下的目录元数据；命令补全仍优先，无候选或空输入回到原 Tab 焦点切换。补全只替换该分段（目录含既有分隔符），支持 ./ ../、绝对路径、~/、引号/反斜杠转义、空格/Unicode 与分段中部光标，且不重复后缀；一次补全通过现有的全局范围替换完成，是一步 undo，图片与 kill buffer 保留，纯无操作不丢 redo。多候选弹出目录优先、Unicode 小写名再原文排序的紧凑列表，最多 50 条，截断显示 “N more — type to narrow”；Up/Down 选择，Enter/Tab 确认不发送，Esc 关闭不改文本，目录关闭后再次 Tab 列出子目录。搜索/帮助/历史搜索/命令弹窗/会话选择器保持优先，F3/F1/会话切换关闭弹窗且不缓存到另一 cwd。已知限制：不支持 `~user`、变量/通配/命令替换；同一路径分段内的混合引号、或光标位于空引号分段内时明确拒绝且不改草稿；替换范围分别取首个字符的原始起点与末个字符的原始终点，保留开闭引号；目录/文件分类会跟随符号链接的目标元数据（不读取文件内容）；跳过非 UTF-8 文件名；为正确排序会在显式 Tab 时读取整个目标目录（仅元数据），随后截断展示。

2026-10-20 渲染文本搜索（#27）：F3 在普通输入／记录界面打开 `transcript_search`，对当前已加载可见记录（含离屏）的渲染行做大小写不敏感匹配；折叠的 Activity／工具／推理正文不索引，展开后重建。匹配以 `RowKey` + 显示单元格范围记录，高亮通过拆分 span 样式完成，不按 `VisualLine.source_start` 反推原文位置；复制与源文本不受影响。命中导航复用 `VisualAnchor`／`Anchor` 源偏移回退，跳跃停止 follow；Esc 恢复原 follow／锚点／焦点。空查询、无匹配、当前/总数有英文提示，当前命中用下划线＋反显（非仅颜色）。搜索状态以显式记录／投影修订号失效，配合查询与宽度重建；布局缓存是否可复用在变更后重建布局时也不被当作新鲜度依据。大小写处理对整行渲染文本与查询统一做 Unicode 小写（含希腊词尾 sigma），再用逐 grapheme 小写边界把命中映射回原始单元格，扩展小写如 İ 仍保留原宽度；渲染中真实出现的符号（包括 ▌）按所见内容参与搜索。高亮先按 `RowKey` 二分到当前行，再走本行命中，避免按全量历史逐字符扫描。已知限制：匹配不跨软换行边界（同一行内），因此这是渲染行搜索而非原文全文搜索。测试覆盖大小写／空／无匹配、扩展小写与 grapheme 单元格、Markdown／代码／表格宽窄回退、导航循环与两种阅读模式恢复、草稿/光标/附件/undo、忙碌 Esc 优先、隐藏正文、流式/最终/重排/折叠失效（含先重建布局再刷新搜索）、粘贴与 F1/会话边界。

2026-10-19 用户消息边界导航（#26）：Ctrl+Shift+Up/Down 在普通记录／输入界面跳转到上一条／下一条真实 User 记录；帮助、输入历史搜索、日志与会话选择器优先。边界取 User 块首个内容行（跳过前导背景留白），因此从长消息内部上移先回到该消息起点，已在起点则选择上一条；Next 取当前顶部记录之后的下一条 User。跳跃停止 follow，复用现有 `VisualAnchor`／`Anchor`（source offset 0）与 `current_top_row`／`record_layout`，resize、新输出与 Activity 收起后仍稳定。跳跃不改动草稿、光标、附件与焦点；无更早／更晚消息时显示英文提示。候选 User 先按现有可见顺序选取，只为当前 User（需要时）和最终目标构建首内容行布局。测试覆盖空／首／尾提示、长多行用户与工具记录、起点与内部、宽窄重排、新输出与 Activity 收起、Ctrl+Shift 双修饰键要求及既有 Alt/普通方向键行为。

2026-10-19 撤销／重做游标语义修复（#35，独立 QA 回归）：`Snapshot` 原先未记录 `preferred_col`，`restore` 又强制置 `None`，使多行草稿在末行末尾上移后，经「编辑＋撤销」再上移会丢失首选显示列（QA 探针实测 control=(0,8) vs edit+undo=(0,1)，PTY 提交文本变成 `a|bcdefgh\nx\nabcdefgh`）。fix commit `d50fddae899581c5a2919f84d16d48ed6937b8bb`（tree `747e029122639870bad35c84c5c611109028c2f7`）把 `preferred_col` 纳入快照：撤销恢复编辑前的首选列，重做恢复其自身的编辑后游标态；普通编辑仍重置首选列，kill buffer 仍独立。QA 原始探针（`/tmp/saki-qa70-evidence/preferred-column-probe.rs`）在修复源上复现为 control=(0,8) == edit+undo=(0,8)，退出 0。修复冻结二进制 SHA256 `bcbb9c14afaf9b6d7cd13634e9603e878f392677ba080723b79482e88c019fa3`；真实 PTY 复跑 QA 场景（`/tmp/saki-tui-35-cursor-fix-evidence/undo_acceptance_cursor_fix.py`，只读复用原始 harness）：6/6 通过，`preferred-edit-undo` 提交文本修正为 `abcdefgh|\nx\nabcdefgh`。新源门禁：`cargo fmt --check` 0、`cargo clippy --locked --all-targets -- -D warnings` 0、`cargo test --locked` 344 通过 / 0 失败 / 6 忽略、显式 ignored 生命周期 PTY 通过。说明：2026-10-18 的作者自测未覆盖该回退；Runtime #58 的 EPERM 偶发仍是独立问题，本次全量运行未复现，不据此改写历史失败。

2026-10-18 输入区撤销／重做（#35，代码 commit `c513e2a0415c1b0fa032a9a3a3ee8b67eadf4fe0`，冻结二进制 SHA256 `2d3fdc0373278a11c254e24b0c952438c1fc55008d28e5b39b19805057e2facb`）：`Alt+U` 撤销、`Alt+R` 重做，仅输入焦点且局部模态优先，不改 `Ctrl+Z`。编辑器持有有界历史（最多 100 条，undo＋redo 保留文本合计硬上限 8 MiB，换行计入）：连续单字符输入合并为一步；移动光标、`set_global_offset`、粘贴或非输入编辑断开分组；粘贴／kill／yank／图片增删各为一步；无操作不丢弃 redo，撤销后新编辑丢弃 redo。`Ctrl+C` 清空可撤销且仍写入输入历史；成功发送、会话切换（含缓存编辑器）与已执行命令重置撤销历史；输入历史召回是可按撤的整草稿替换。超预算时：单次编辑的前状态超预算则重置并跳过历史；撤销／重做时当前草稿无法存入预算则保持草稿不变且不可用。真实 PTY（`/tmp/saki-tui-35-evidence/undo_redo_pty.py`）9/9 检查通过：typing→Alt+U/Alt+R、移动断开分组、`Ctrl+C` 清空→Alt+U 恢复，且同一轮 mock 记录 0 个模型请求（未提交消息）。门禁：`cargo fmt --check` 与 `cargo clippy --locked --all-targets -- -D warnings` 通过；TUI 子集 219 通过 / 0 失败 / 5 忽略。**已知限制**：全量 `cargo test --locked` 出现 1 个与本改动无关的 Runtime/tools `bash_preserves_stdout_stderr_write_order` EPERM 失败（Runtime #58），定向复跑通过；该失败不由本 TUI 改动引入，本轮不声称全量测试全绿。

2026-10-16 最终候选集成树（#25）：代码 HEAD `9ddeab177ed0e0331b175886f9dbd4cdf17a6737`，tree `abd3d6249ac509ff3b04ab82b4815a83f47fef59`（parents `82c6c09` + `e9fdb7f`；ancestors 含 runtime `f5aa56684671b1972a692712f96a91b397da042e` 与 TUI `e635bb545ec60d039041e8703a6c9002057e0e79`）。组合门禁全部通过：`cargo fmt --all -- --check` 0；`cargo clippy --all-targets -- -D warnings` **0**；`cargo test --locked` **313 通过 / 0 失败 / 6 忽略**；`cargo test --locked --example growth_experiment` **42 通过**；`cd desktop && go test ./...` 通过。独立真实 PTY 在该 exact HEAD 上 **9/9 场景 / 121/121 断言**通过（`session_search`、`session_search_exact_open`、`session_search_narrow_unicode`、`history_search`、`history_search_modal`、`history_search_busy_esc`、`help_esc_cancel`、`help_narrow_render`、`search_then_help_then_interrupt`）。QA 冻结二进制 `/tmp/saki-tui-search-qa/Saki-clippy-final`，sha256 `a73628f51ca3db9e7673d1885499988f0e86d4ce49e7d6375ca1ceba30a5b809`（独立于作者路径 debug 二进制 `3de90616f50b2e56f5c94a3565ae3ba890d385db1c6c9f0d0dea3238899980af`）；报告 `report-25-final.json`，见 QA 永久终验评论 [#25 comment](https://github.com/HeimaoLST/Saki/issues/25#issuecomment-6207358437)。baseline 组合修复树 `c558cf8e0a26f1ed4fa5067d4d493821482ff426`/tree `dec634b5755bfa5b910accb4990a5268bf432de2` 亦全绿（285 主测试 + 42 实验测试，clippy 0）。[#40](https://github.com/HeimaoLST/Saki/issues/40)（PR #45）清除了 `src/frontend/**`、`tests/tui/**` 的 Clippy 诊断，runtime 侧由 #39（PR #42）完成；#25 记录的基线 clippy exit 101 属历史事实，**仅对该候选集成树被取代**，不代表 `dev`/`master` 或 runtime 单独分支已合并或全绿。验收平台为 macOS arm64 + 受控 HTTP/SSE mock；本轮**没有**新增真实模型调用或物理 OS 剪贴板验证。

2026-10-16（历史，pre-clippy-integration 功能栈）会话选择器搜索、输入历史搜索与上下文帮助（代码 commit `9bbf47c`，冻结二进制 SHA `f9fcad69adee3f7f9bc59db53e50ce9f5f4cf3a0b4e353e1ac6ea4ff34778f81`）：组合版本 `cargo test` **313 通过 / 0 失败 / 6 忽略**，`cargo fmt --check`、`cargo build` 与 `cd desktop && go test ./...` 通过；本轮为文档追加，代码与二进制未变。独立真实 PTY 在最终组合二进制上 **9/9 场景、121/121 断言通过**（报告 `/tmp/saki-tui-search-qa/report-23-final-stack.json`；QA 评论见 [#34](https://github.com/HeimaoLST/Saki/issues/34#issuecomment-6197099561)，root 复核见 [PR #38](https://github.com/HeimaoLST/Saki/pull/38#issuecomment-6197056801)）：`/` 会话选择器本地元数据搜索、过滤后精确 ID 打开与无匹配空状态、窄字段最长可用后缀 + grapheme Backspace；`Ctrl+R` 输入历史搜索（接受不发送、第二次 Enter 提交精确文本／图片、取消恢复原草稿与光标、模态与粘贴路由、busy Esc 本地优先）；运行中帮助第一次 Esc 只关帮助不取消、第二次 Esc 中断且保留草稿；以及 `#22`+`#23` 组合过渡。验收平台为 macOS arm64 + 受控 HTTP/SSE mock；本轮**没有**新增真实模型调用或物理 OS 剪贴板验证。`cargo clippy --all-targets -- -D warnings` 仍为既有 [#25](https://github.com/HeimaoLST/Saki/issues/25) 基线失败（exit 101，本迭代前端改动无新增诊断），因此不声称所有门禁为绿。

2026-10-15 自动标题与 Esc 取消：组合串行全量 **282 主程序 + 40 成长实验**通过，`cargo fmt --all -- --check` 通过。冻结二进制 SHA `63b74e217c5d239e…` 上独立 5 场景 PTY 验收 5/5 通过（报告 `/tmp/saki-title-cancel-qa/report.json`）：自动标题一次尝试、失败不致命、流式取消保留有序部分文本与草稿、bash 进程组含后代终止、等待响应取消与空闲／帮助／选择器 Esc 优先级、陈旧与重复取消无害。真实 Magpie 网关 `opencode-go/deepseek-v4.1-flash` 现场冒烟通过，无工具调用，自动标题为 `Simple Greeting Request and Response`（`/tmp/saki-title-cancel-qa/live_report.json`）。界面保持英文标题、仅新会话可自动命名、旧 metadata 的显式标题保留；取消不是事务：在途工具已开始的文件写入可能完成或留下半写状态，不作回滚承诺；被取消／未完成轮次的公开推理仅界面显示，不持久化为 provider payload，正常完成轮次的推理持久化保持不变。

2026-10-12 底栏精简：101 项 TUI 回归通过，覆盖比例／容量靠左、模型靠右、不同宽度操作入口、未知值、动态容量与 Help 详细信息；格式检查通过。此次仅调整展示布局与格式，复用已有计数与事件，不改自动压缩策略。

2026-10-11 普通拖选与粘贴修复：串行全量测试通过（主程序 158 项、成长实验 34 项），格式／编译／构建通过。新增回归覆盖宽字符半格拖选、组合字形、高亮、反向跨行复制、复制保留草稿、元数据更新与 resize／内容变化、滚动中止选择，以及从记录／帮助粘贴返回输入。真实 Ghostty → tmux → SAKI 验证普通无修饰键拖选自动复制、原生 Shift 选择与复制、⌘V 多行粘贴、滚轮／方向键、正常退出；完整系统剪贴板内容与窗口焦点恢复。证据位于 `.saki/verification/clipboard-vision/`。同期将本地 `.env` 的模型切为原生图片入口 `opencode-go/deepseek-v4.1-flash`，实际图文验证与旧入口代读的区别见 [图片输入](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/design/multimodal-input.md#原生视觉与网关代读)。

2026-10-11 状态栏与英文文案调整：格式、编译、构建和串行全量测试通过（主程序 154 项、成长实验 34 项）。新增行为验证包括启动时模型／真实配置容量、上游 usage 校准后的工具结果增量、无 usage 响应继续估算、compact 后立即替换占用、未知值、不同宽度状态栏以及 metadata 不触发未读提示。独立 tmux 对模型／上下文、帮助、执行中复制、回看和新结果、24×8／12×4 缩放通过；输入、四工具／compact／API 失败恢复和图片剪贴板回归另行通过。证据聚合于 `.saki/verification/tui-statusline/`，本轮使用受控 HTTP 响应与实际工具／终端交互，没有追加真实模型请求。

2026-10-10 的 UX 调整以 Ratatui TestBackend 覆盖按需帮助、图片草稿／光标／焦点／锚点保留、问号语义、帮助滚动和微小窗口、按钮命中、新输出与复制提示到期，以及单行工具摘要保留完整参数／复制原文。`cargo fmt --all -- --check`、`cargo check --all-targets`、`cargo build --bin Saki` 通过；`cargo test --all-targets -- --test-threads=1` 主程序 151 项、成长实验 34 项通过。

独立 tmux 验收通过实际帮助点击／F1／问号／Escape、帮助与长命令复制期间真实 bash 等待状态、三秒反馈自动到期、新工具结果不夺走阅读锚点、点击回到最新、24×8 与 12×4 缩放。复用受控 HTTP 回归分别通过输入编辑与历史、工具／compact／API 失败恢复、图片路径与系统剪贴板输入。实际 terminal captures、ANSI 截图、脚本和结果聚合于 `.saki/verification/tui-ux-implementation/`；没有追加真实模型请求，界面交互通过受控响应和实际工具进程验证。

默认并行全量测试中，既有 `bash_drains_both_pipes_and_large_output_is_complete_and_readable` 再次出现输出行数与 preview 末尾标记断言波动；单独测试与串行全量通过。2026-10-14 独立最终全量尝试中该测试在 `tests/tools/mod.rs:244` 的 `preview.contains("FINAL_MARKER")` 断言失败（该次主程序 230 通过、1 失败、2 忽略）；定向复跑通过，随后最终全量重跑全部通过。工具实现与测试的 SHA256 与基线一致；源码复核认为测试假定了异步合并 stdout／stderr 的次序（包括有界末尾中的标记与最终日志行），这是波动的合理解释，不是本轮新复现的 TUI 缺陷。工具及其测试不在本次 TUI style 实验范围内，不声称这项既有波动已修复；该观察保留在验证记录中。失败日志已保留，此前验收结果继续保留在下方。

- 独立执行 `cargo fmt --all -- --check`、`cargo check --all-targets`、`cargo build --bin Saki`，均通过；正文对齐后的 `cargo test --all-targets`：主程序 108 项、成长实验 29 项通过（首版为 84 + 29，视觉调整为 94 + 29，滚动修复为 104 + 29）。
- 实际二进制在独立 tmux 会话通过 Unicode 组合字符编辑、bracketed paste、忙碌草稿、12×4 缩放恢复、工具非零退出、API 401 后下一轮和 overflow compact 验收。压缩时没有清除界面历史或夺走阅读位置。
- macOS 系统剪贴板实测消息、代码正文和命令原文复制；实际 bash 生成十万字符单行与 300 条日志，浏览到 `LOG_END`、回翻、Esc 恢复原 viewport 均通过。
- Magpie `opencode-go/deepseek-v4-flash` 在隔离目录实际调用 read/bash/edit/write：Rust 测试修复前 exit 101，修复后 exit 0；另行运行 rustc 独立确认通过，第二轮消息与退出恢复也通过。该次配置为 32768 context、4096 output、1024 summary。
- 直接引用生产终端模块的隔离 harness 验证正常 Drop、panic 和初始化写入失败；shell 恢复，`stty -g` 前后相同。

原始脚本、结果与终端 capture 位于 `.saki/verification/tui-implementation/`，阶段审查见其中 `supervision.md`。受控 HTTP 用于确定性边界测试，与真实 Magpie 任务分开记录。一个模型试跑曾在失败命令后追加 `echo`，工具因 shell 最后命令成功而正确返回 0；后续验收输入明确保留测试退出码。界面展示实际 shell 状态，不从日志关键词猜业务是否成功。

视觉调整另存于 `.saki/verification/tui-polish/`：包含基线、前后终端 capture、格式／构建／测试结果和对比 HTML。复用独立终端与内容脚本再次通过输入、阅读锚点、compact、工具失败、复制及长日志验收；另以 120／48 列检查中文、多行输入、emoji、组合字符、背景覆盖和选择标记。对比 HTML 来自实际 ANSI capture，浏览器字体与终端默认背景属于展示选项，不能代替终端交互验收。本次没有追加真实模型调用。

后续用户实测发现滚轮没有响应、方向键无法逐行浏览单条长回复。已接入终端鼠标捕获与 `Event::Mouse`，调整记录区方向键，复用原有阅读锚点滚动。同时修正 Markdown 合成空白与真实文本 segment 合并造成的字节偏移失真，补充段落换行后软换行锚点回归。独立 `scroll-smoke.py` 在真实 tmux 中注入 SGR 滚轮，经 EventStream 验证长回复的滚轮／方向键／按页导航、草稿与焦点保留、消息选择、首尾跳转、新回复不抢位置；正常退出的鼠标捕获关闭与 `stty` 恢复通过。生命周期 harness 另验证 panic 和初始化失败也恢复鼠标模式。脚本与结果仍聚合在上述两个验收目录。

正文对齐后再次通过核心与滚动 smoke。扩展原有内容 smoke：从 macOS 系统剪贴板读取包含中文、emoji、组合字符、CRLF／CR 的多行文本，经真实 tmux bracketed paste 进入 EventStream，确认保持草稿、不自动发送，Enter 后模型请求内容准确且换行归一为 LF；消息、代码和工具命令原文复制仍通过。tmux 测试使用 `paste-buffer -p -r -S` 保留原字节，避免默认 LF→CR 转换让 CRLF 变成两个 CR。测试原剪贴板会恢复。前后视觉证据为 `tui-polish/alignment-comparison.html`。

用户进一步要求实际测试 tmux。已用当前 Ghostty 1.3.1 的 [AppleScript 接口](https://ghostty.org/docs/features/applescript) 新建独立窗口，附着独立 tmux 会话运行实际二进制，完成 Ghostty→tmux→SAKI 全路径验证：鼠标滚轮、记录方向键、普通拖选被捕获、Shift 拖选进行原生选择、包含中文／emoji／组合字符的可见文本复制、多行原生粘贴保持草稿及 Enter 后请求原文。复制与粘贴调用当前 ⌘C／⌘V 绑定对应的 Ghostty 原生 action（`copy_to_clipboard:mixed`／`paste_from_clipboard`），没有用 System Events 模拟物理按键；Shift 拖选使用 Ghostty 鼠标事件接口。原剪贴板与窗口焦点恢复，测试窗口／会话关闭，用户原会话没有输入或重启。结果见 `tui-polish/ghostty-smoke-result.json`，脚本 `ghostty-smoke.py`。

当前验收平台为 macOS + tmux。中文文本、emoji 与组合字符已验证；真实输入法候选窗、其他终端及平台尚未验证。表格排版、Activity 分组、主题切换、窄屏底栏、bracketed paste、Ctrl+C 与终端恢复到 2026-10-14 已在隔离 tmux 会话中通过真实二进制验收；原生主题实现改由 Termina 统一输入读取：启动采样 OSC 10/11 并订阅 mode2031/CSI ?997，正文继承终端前景，衍生 palette 从采样 fg/bg 推导；palette 变化清空缓存布局，会话切换保留 live 颜色。物理 OS／Ghostty 外观切换与系统剪贴板仍未验证；本轮已通过真实 Magpie 网关的问候与自动标题冒烟，但不声称覆盖完整物理 Ghostty 交互或真实模型工具链；Termina 0.3.3 的 `rgba:` 回复与 ignored-SIGHUP 断开场景是已知边界；代码语法高亮仍留待后续；主动取消已实现并验收（见“实际验收”）；会话区跨 viewport 选择已实现，日志跨文件页选择不在范围内；会话恢复与选择器、本地命令和补全现已实现，见[会话管理](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/design/session-management.md)与[命令系统](command-system.md)。

### 输入框优化验收

2026-10-08 按 [Codex 输入研究](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-ui.md#7-codex-输入框的编辑手感) 完成快捷编辑、当前进程输入历史与视觉行导航。生产变更限于 `src/frontend/tui/`；Editor 负责文本、光标、单条删除缓冲和唯一换行布局，App 持有 InputHistory，前端继续通过原通道提交消息，Agent／Conversation／worker 不增加 UI 依赖。

审查修正了软换行片段末尾与下一片段首位重合造成的光标停滞、resize 后旧目标列越界，以及标点上的组合音标被按词删除拆开的边界。永久回归覆盖 Unicode、跨行删除与恢复、无操作删除不覆盖缓冲、历史召回后编辑、忙时清空、日志关闭优先级、满宽末尾与宽窄切换。`cargo fmt --check`、`cargo check --all-targets`、`cargo build` 与全量测试通过：128 个主程序测试、29 个实验模块测试。

独立 tmux 输入验收发送 9 个受控 HTTP 请求，逐次比较实际提交原文：字符／词／行编辑、Ctrl+J、多行换行归一、Ctrl+Y、Ctrl+P/N 与 ↑/↓ 召回、普通及修改后的草稿不被覆盖、忙时 Ctrl+C 清空后仍能继续使用、视觉换行与目标显示列、中文／emoji cell 宽度、恰好满宽的末尾光标、Meta 按键事件，以及空输入忙时等待完成再退出。核心回归另验证真实 write/bash、overflow/compact、API 失败后下一轮、阅读锚点与窄窗；内容回归验证系统剪贴板多行 Paste、原文／代码／命令复制和 55 页大日志。测试使用实际二进制与 tmux EventStream，模型回复由本地 HTTP fixture 控制，没有新增真实模型调用，也没有对用户正在运行的会话输入或重启。

本轮证据聚合于 `.saki/verification/tui-input/`：`input-smoke-result.json`、`tui-smoke-result.json`、`content-smoke-result.json`、编译检查日志及 `supervision.md`。首次独立全量测试中，既有 `bash_drains_both_pipes_and_large_output_is_complete_and_readable` 的 stdout/stderr 合并输出按行计数断言出现 19997／20000 差异；定向与最终全量复跑通过。首次日志仍保留为 `tests-initial.log`，本轮未修改工具实现，不声称这项波动已修复。

会话恢复会从已保存的用户消息重建输入历史；未发送或 Ctrl+C 清空的草稿仍只保留在当前进程。外部编辑器与长粘贴占位仍留待后续需求；Ctrl+R 输入历史搜索与通用撤销／重做（Alt+U／Alt+R，仅输入焦点）已实现；任务取消已实现并验收（运行中 Esc）。

## 四种布局候选

### 下一版交互提案：减少常驻操作提示

2026-10-09 用户反馈：常驻快捷键列表显得拥挤，希望参考 Codex 的 UX，同时形成自己的风格。以下保留当时提案；2026-10-10 已按推荐的工作记录方向实施，实际行为见上面的“消息与底栏的视觉调整”。

建议沿用“工作记录”布局，把内容、状态反馈和操作帮助分开：

- 正常界面只保留简短身份／状态及一个“帮助”入口。发送、换行、复制、滚动等按键不常驻。模型、token 计数等放到按需查看的会话信息。
- 开始执行或进入回看时，提供当前活动或“有新输出”的直接反馈。复制成功等临时反馈不能替换运行状态，防止用户误以为任务已经停止。
- 完整帮助由用户主动打开，按“输入”“记录”等行为分组，位于输入上方；保留原草稿和阅读位置。建议 F1，以及空草稿时的 `?`，已有文字或附件时 `?` 继续作为文字。点击“帮助”需要该区域的鼠标命中处理，本次已接入。
- 维持青绿强调色、用户背景块、统一正文对齐和有限边界。状态用文字表达，颜色辅助。Assistant 不追加逐条角色标签。

已制作三个可交互方向用于比较：**工作记录**（推荐，保留紧凑工具依据与简单底栏）、**轻量对话**（更少常驻标识，帮助的可发现性较弱）、**工作区**（固定目录信息，适合多项目切换，占用更多空间）。它们都移除常驻快捷键列表，帮助按需展开；不是三个实现计划。设计稿可以切换场景和图片附件。当前 HTML 交互只用于讨论，不证明终端键盘、点击区域或帮助层已经实现。

依据是本机 Codex 0.161.0 对应固定版本的 [footer.rs](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/tui/src/bottom_pane/footer.rs) 和 [shortcut_overlay.rs](https://github.com/openai/codex/blob/979011409de0a60b52f179721948e65531d26144/codex-rs/tui/src/bottom_pane/shortcut_overlay.rs)。源码区分输入为空、有草稿和临时提示，并将完整快捷键放在独立帮助层。SAKI 借鉴这个信息层级，保留自己的记录、配色和文字风格；不引入尚未支持的排队或会话分叉语义；取消与本地命令现已支持，见“实际验收”。

### 流式对话

```text
标识与目录
对话 → 工具 → 对话 → 工具 → 结论
输入
状态
```

交互成本低，适合短任务和窄窗口。工具逐项折叠，失败默认呈现关键错误。连续几十次工具调用时，最终答案与最初问题之间距离很长。

### 工作记录：推荐作为主体

```text
标识与目录
本轮用户请求
SAKI 的公开说明
  │ 执行记录：调用、结果、失败
  │ 可逐项展开
SAKI 的结果与判断
输入
状态
```

每轮任务形成一个可回看的工作单元。视觉上使用一条细线聚合执行记录，正文与结论保持完整宽度。单元标识是当前任务的组织方式，不暗示已经具备跨重启身份或历史恢复。

工具严格按实际发生顺序展示，保留公开中间说明；首版不自动生成“调查／修改／验证”等语义阶段，也不为布局额外请求 LLM。题目可以沿用用户请求的首行；长内容省略显示时仍能查看原文。

失败是历史事实。后续任务成功不删除此前失败；“已修复”只能来自明确后续验证或公开说明，不能把任意 shell exit 0 当成业务验证通过。此前 Magpie 实测出现命令末尾 echo 掩盖退出码，详见 [builtin tools 验收](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/design/builtin-tools.md)。

### 双栏工作台

```text
标识与目录
对话与工具列表 │ 选中调用的输出／diff
输入
状态
```

适合宽屏工程任务。选择一项工具，旁边显示其输入、输出和状态；输入始终属于同一会话。窄屏转成上下布局或临时详情页，避免继续压缩正文。

初版详情基于已有工具数据。read 可以显示文本页；bash 显示捕获输出；edit 当前只返回 applied 数量，不直接返回 diff。因此示意中的精确 diff 若进入正式实现，需要有可靠的变更数据来源，不能从 applied=1 伪造 diff，也不能把工具调用参数直接当成已写入事实。

### 底部检视抽屉

```text
标识与目录
对话
按需展开的工具详情
输入
状态
```

适合窄分屏，也可以作为工作记录的详情表现形式。抽屉展开时保留当前选择与输入草稿；屏幕高度不足时，用临时详情视图替代同时挤压所有区域。不能靠让字号变小来容纳内容。

## 推荐组合与适配

用户初步偏好工作记录，后续以它作为第一版主体方案；详情的具体展现根据终端空间适配。其他候选保留作为比较依据，暂不引入四套正式布局配置。

| 实际空间 | 建议表现 |
| --- | --- |
| 窄 pane | 单栏，工具原位展开；长详情临时查看 |
| 一般终端 | 单栏工作记录，详情可在下方打开 |
| 宽终端 | 用户打开详情后才出现右侧检视区 |
| 高度很小 | 压缩次要状态，详情临时全屏，确保输入仍可使用 |

初始可用 120 列附近作为侧边详情候选阈值，且正文和详情各保留约 60 列；这是待校准假设，实际还需要边框、留白和 CJK 宽度，不能照搬浏览器像素断点。宽屏也不强制常驻侧栏。

空间布局与滚动策略分开决定。如果第一版优先固定输入、任意展开和工具检视，可以先评估程序管理 transcript 的 fullscreen 路径；复制与滚动锚点必须同时可用。regular/inline 适合终端原生 scrollback，但历史重绘、折叠和退出后的文本保留需要单独处理。尚未承诺同时实现两种渲染策略。

## 视觉语言

- 推荐青瓷方向：继承终端明暗背景，用低饱和青绿色强调 saki、当前选择与输入分隔线。
- 墨纸方向作为审美备选：暖灰背景、柔和陶土色强调。同一套布局只改变视觉语气。
- 终端原色方向保留 ANSI 与用户终端主题的兼容性，作为后续能力不足时的回退设计依据。
- 角色名称、缩进、留白与细线形成层次；错误还应有文字与符号，状态不只靠颜色表达。
- 采用普通等宽字体和可回退字符；不要求 Nerd Font。避免常驻大 logo 占用窄 pane。
- context 未经响应 usage 校准时标注估算；不展示无依据的进度百分比、价格或人格数值。

## 技术栈与组件决策

2026-10-06 核对 crates.io 已发布版本与官方资料，并用独立临时程序验证兼容性。用户确定采用 Ratatui，优先原生组件；原生能力无法很好满足具体需求时，维护 SAKI 自己的组件以支持后续开发。尚未修改项目 Cargo.toml 或 Cargo.lock。

| 职责 | 推荐 | 作用与边界 |
| --- | --- | --- |
| 渲染、布局、基础组件 | Ratatui 0.30.2 | Layout、Block、Paragraph、List、Scrollbar、Tabs 等；组合成工作记录布局 |
| 终端输入与模式控制 | Crossterm 0.29.0 | 键盘、粘贴、鼠标、窗口缩放、raw mode、alternate screen；使用单一事件读取路径 |
| 异步运行 | 已有 Tokio 1.x | UI 与 Agent 独立运行，通过 mpsc 交换任务与展示事件 |
| 多行输入 | 基于原生文本组件评估 | 编辑与光标状态、中文宽度、发送／换行和草稿生命周期由 SAKI 明确管理 |
| Markdown | 原生 Text／Span／Paragraph 承载展示 | Markdown 解析和代码高亮单独评估；需要解析依赖不意味着引入整体组件框架 |
| 可变高度记录列表 | 原生组件组合，按需实现记录视图 | 明确跟随底部、折叠后锚点与长消息的可见区域渲染 |
| 有界详情滚动 | 原生 Paragraph／Scrollbar | 不把全部长期历史渲染成一个大缓冲 |

已研究的 ratatui-textarea、tui-markdown、tui-widget-list 保留为能力与实现参考，并未确定引入。若后续需要专用依赖，再依据实际缺口评估。旧 tui-textarea 0.7.0 依赖 Ratatui 0.29；不能直接按旧教程加入 0.30 技术栈。源码与版本证据见 [Terminal UI 组件研究](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-ui.md#5-rust-技术栈与组件核对)。

第一版使用原生组件组合工作记录、工具组、输入区与状态栏。应用状态留在 TUI 中；只有具备明确复用价值的渲染和交互职责才提取为自己的组件，首先与 TUI 源码聚合维护。出现多处实际复用后再决定是否独立成组件库，不预先设计通用控件框架或发布独立 crate。

Ratatui 只负责显示；Conversation 继续管理模型上下文，TUI 保留可阅读的原始工作记录。compact 不应让屏幕上的早期任务凭空消失。Agent 发出消息、工具开始／结束、压缩和失败事件；只有 UI 写终端。首版采用非流式等待；2026-10-13 已接入增量正文。

Crossterm 官方禁止把 EventStream 与 read/poll 混用。推荐 UI 使用 EventStream 配合 tokio::select!，消费流时增加 futures-util::StreamExt；输入组件只处理传入事件，不自行读取终端。Tokio 已在项目中，避免另起一套异步 runtime。

### 已做的兼容性验证

此前的候选组合小样使用 TestBackend 在 80×24 cell 区域同时绘制 Markdown、ListView 与 TextArea，直接传入 Crossterm KeyEvent 并检查中文多行编辑结果。编译与执行通过；依赖图中 Ratatui Core、Widgets、Crossterm Backend 与 Crossterm 均只有一个解析版本。这是第三方候选组合的验证，不代表已采用这些专用组件，也未验证未来的 SAKI 自有组件。

该组合所用 Ratatui 和 Markdown 的声明最低 Rust 版本为 1.88；当前本地 rustc 1.99.0 满足要求。原始小样与解析记录保存在 .saki/verification/tui-stack/，不属于生产功能。

这项检查证明类型与基础组件能够一起使用，尚未验证该检查当时的真实终端、输入法、长历史性能、取消或已有 Agent 的实际接入（真实终端与取消后来已在“实际验收”中覆盖）。

### Codex 参考：组件职责与交互机制

已检查 Codex 固定源码版本，详细证据见 [Codex 组件研究](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/docs/research/systems/terminal-ui.md#6-codex-的自有组件与交互机制)。下列是待验证的实现建议，维持 Ratatui 原生组件优先的决定。

- 将文本编辑状态与提交任务分开管理；中文编辑按 grapheme 与 cell 宽度处理，粘贴中的换行不触发提交。
- 将工具调用作为可更新记录，以 call_id 路由开始与完成事件，紧凑摘要和展开详情使用同一份结果。
- 区分跟随最新与回看；阅读锚点关联记录及内容位置，resize 后重新映射。
- 保留原文作为复制和派生排版的依据，细线、状态符号和折叠提示属于展示装饰。
- 由输入、任务事件、resize 和活动计时器请求重绘；布局缓存按变化失效，普通帧不重排全部历史。
- 集中处理焦点路由与终端恢复；切换详情保留输入草稿。

初期可用普通 struct／enum 表达记录视图、输入区、工具项与状态栏。公共组件接口在出现复用后提取。Codex 的完整组件 trait、命令／附件／Vim 系统与终端 fork 不作为第一版的依赖要求。

## 初版实现计划

已有布局对比、技术栈核对、兼容性小样和 Codex 组件研究，形成了下面的首版计划；用户随后明确授权 Pi 实现，Codex 监督验收。以下保留设计时的范围与阶段划分，实际完成情况以上面的实现与验收为准。

### 范围与交互默认值

| 项目 | 首版提案 |
| --- | --- |
| 屏幕与布局 | alternate screen，由程序管理历史；单栏工作记录，固定输入和状态，工具原位展开；侧栏／抽屉属于后续增强 |
| 主题 | 继承终端背景，青瓷强调色，角色／状态用文字与符号区分；不要求特殊字体 |
| 输入 | 自有多行编辑状态；支持中文、grapheme 边界、显式粘贴、字符／词／行快捷编辑、单条 kill buffer、输入历史与按软换行的视觉移动；输入高度有上限，长草稿在输入区内滚动 |
| 发送与换行 | Enter 发送，Alt／Shift+Enter 或 Ctrl+J 换行；多行粘贴保留换行，不自动发送 |
| 焦点与导航 | Tab 在输入与记录视图之间切换；滚轮／PageUp／PageDown 回看；记录视图方向键逐行滚动，Shift+方向键选择记录；输入区 ↑／↓、Ctrl+P／N 在软换行与历史之间复用；Alt+方向键在任意焦点逐行滚动；记录视图 Home／End 跳对话首尾，Ctrl 组合键全局可用；Enter 在记录视图中只展开／收起工具 |
| 运行中输入 | 可编辑草稿；Enter 保留草稿并提示等待当前任务完成，不提交、不自动排队；Ctrl+C 在有草稿时只保存并清空，不请求退出 |
| 退出 | 空输入时 Ctrl+C／Ctrl+D 空闲退出；运行中则等待当前轮完成并显示等待状态；有草稿时 Ctrl+C 先清草稿。运行中取消改用 Esc（见上表），不直接 abort worker。Ctrl+C 不宣称已取消 |
| 历史 | 按轮组织用户输入、响应正文、工具记录与结论；call_id 定位工具；跟随最新与回看两种状态；compact 不删除屏幕记录 |
| 工具详情 | 显示实际参数、结果、错误、退出码／超时、截断标记；有 output_path 时提供有界日志浏览；edit 显示实际返回结果，不生成伪 diff |
| Markdown 与复制 | 基础段落、标题、列表、强调和代码块；复杂表格先保留原文；提供选中消息／代码块／工具命令的原文复制入口；会话区跨 viewport 文本选择已实现（日志跨文件页排除）；语法高亮后续评估 |
| 当前平台 | 首先验收 macOS 终端与 tmux；当前 bash 工具要求 Unix，不将首版宣称为完整跨平台产品 |

普通字符输入（包括 `/`）保持文本语义；命令解析与补全等到 command 系统设计后再接入。运行中等待退出可能持续到模型和工具完成或超时，首版明确显示此状态，不声称已取消（取消请用 Esc）。

### 现有代码的接入范围

[Agent::handle_user_message](https://github.com/HeimaoLST/Saki/blob/00298ae22bb95afc11366597e517a3b1ceba21e7/src/agent/mod.rs) 保留唯一模型／工具执行循环，移出终端输入和打印职责。UI 在现有 Tokio runtime 上驱动单个 Agent worker，以任务 channel 交付输入，以事件 channel 接收展示数据。Agent 不依赖 Ratatui；Conversation 继续负责 usage、compact、overflow recovery 与当前 history journal。

需要覆盖的事件是响应正文、工具开始／完成、压缩结果、usage 和轮次完成；任务错误在 worker 返回边界统一送到 UI，避免每一层重复生成错误展示。轮次开始可由 UI 接受输入时建立。首版非流式请求期间显示等待；后续流式实现通过同一事件边界接入。

当前 `handle_user_message` 将 assistant_text 覆盖为最后一条消息，并且只有没有工具调用时才打印。接入时应按响应中的实际顺序发送每条正文，确保工具调用前的公开说明与多条正文都能进入记录。这是已有输出逻辑的具体接入修正，不另写一个 TUI 专用 Agent loop。

工具事件保留原始结果 Value；TUI 在一处按工具契约生成摘要。bash 的非零 exit_code／timed_out 和工具执行错误都应呈现为失败；命令正常退出只显示执行完成。工具调用参数 JSON 无法解析时，同一 call_id 对应的记录仍必须结束并显示错误。

[main.rs](../../src/main.rs) 中现有的模型、容量、会话目录与工具初始化需要保留，依照下面的模块边界移入应用启动模块。启动阶段错误在进入 raw mode 前正常报告；进入 TUI 后通过统一绘制入口显示错误。终端初始化、恢复和 panic 处理集中在一个模块，正常退出使用同一路径。

用户在实施交接时进一步明确模块边界：main 保持简单，TUI 属于独立模块，没有 TUI 也能运行 Agent loop。按这一要求调整上述启动职责：main 只调用应用启动入口，单个 app 模块集中现有配置／会话／工具初始化、channels 和收尾；Agent 与 worker 独立于界面，TUI 和纯文本适配各自处理输入与展示，均使用同一消息／事件契约。TUI 不持有 Agent、Conversation 或 ToolRegistry，不执行模型／工具业务。纯文本适配作为可用入口保留，不是需要删除的临时实现。

入口选择建议集中在 app 一处：交互 TTY 默认 TUI，非 TTY 默认纯文本；SAKI_UI=console/tui 可以显式选择。该建议是启动配置，不引入斜杠命令解析或通用 presentation 框架。Agent 内没有 UI 模式分支，单独调用 handle_user_message 仍可运行完整业务循环。

源码聚合到 `src/frontend/tui/`，测试聚合到 `tests/tui/`。起步可按事件循环／应用状态、输入编辑、记录与工具展示、终端生命周期分文件；Markdown、主题或工具详情在出现独立复杂度时再拆分。Agent 事件声明与 Agent 职责聚合，暂不建立通用 UI 框架或独立组件 crate。

### 实施顺序与阶段验收

| 阶段 | 实施内容 | 阶段完成标准 |
| --- | --- | --- |
| 1. 执行与展示解耦 | 移出 Agent 的 stdin／println；将正文、工具结果、压缩与完成接入事件 | 现有模型／四工具／overflow 测试仍通过；确定性场景验证正文顺序、call_id、失败后能开启下一轮；compact 前的展示历史仍在 |
| 2. 终端与输入 | 加入基础依赖、单一 EventStream、四区域布局、自有多行输入、草稿／焦点管理 | 真实终端可发送一条输入；中文与组合字符编辑、多行粘贴、窄窗口和正常退出成立；初始化失败和 panic 恢复终端 |
| 3. 工作记录 | 实现工具状态、展开详情、跟随／回看、内容锚点和 resize | 受控工具链中运行状态可见；完成不重复记录；上滚后新结果不抢位置；折叠和缩放保留合理阅读位置与草稿 |
| 4. 可读内容与证据 | 基础 Markdown、原文复制、有界日志浏览、布局复用与青瓷主题 | 代码可读可复制；装饰不进入复制内容；截断入口有效；大输出与巨长单行只加载／排版所需预览或有界页 |
| 5. 完整闭环 | 默认程序入口、真实网关任务、工具失败与 compact 场景、tmux 验收 | cargo fmt／check／test 通过；真实模型完成用户输入→工具→结论→下一轮；缩放、复制、退出与终端恢复通过人工验收 |

阶段 1 的事件验收可以复用现有 Agent scripted HTTP 场景，不引入另一套执行模拟框架。组件行为优先使用 Ratatui TestBackend 与明确的输入事件；异步状态采用 channel／受控调度，避免靠 sleep 掩盖时序问题。

第一版完成标准是可用于现有 agent loop 的交互入口，包含输入、真实工具状态、可回看证据和下一轮任务。首版计划将流式 API、命令系统、主动取消、消息排队、跨重启恢复、磁盘清理、侧栏与通用组件发布留给后续需求；流式 API、会话恢复和本地命令已于 2026-10-13 实现，主动取消与自动标题已于 2026-10-15 实现并验收。

### 实现前的验证约束

研究材料只用于确定实现方向，不能证明自有编辑器、阅读锚点、系统剪贴板或终端组合键的实际效果。此前兼容性小样采用第三方候选组件，不能作为自有组件验收；HTML 对比稿不能代替终端验证。实际实施在阶段 2–3 就完成了独立 tmux 输入检查，随后再验收复制、日志与模型任务。

工具数据已能支持首版摘要与有界详情，无需先改四工具协议。Markdown 解析、Unicode 编辑和剪贴板的专用依赖在对应阶段按具体契约核对版本；基础依赖仍是 Ratatui 0.30.2、Crossterm 0.29.0、已有 Tokio 与 EventStream 消费所需的 futures-util。

## 需要验证的交互契约

1. 回看历史时新输出不抢走位置；用户主动回到底部才恢复跟随。
2. 选择工具与输入焦点分开；详情切换不发送消息，不丢失输入草稿。
3. 全量日志有明确入口，截断提示保留；TUI 不加载整个超长日志到可见区。
4. 复制正文、命令和代码不含装饰标记；鼠标捕获与终端原生选择的冲突需实测。
5. 缩放与主题切换后，选中对象、草稿和阅读位置尽量保留。
6. 在首段增量到达前显示等待；正文增量更新同一记录，完成时不重复显示。断流保留可见片段并报告失败，不执行未完成响应的工具。
7. 首版发送／换行与运行中草稿按上面的默认值验收；用户取消已实现并验收（Esc、恰一个中断终态、保留有序流式部分与草稿），运行中补充指令仍未实现。

## 对比稿与验证边界

四个候选使用同一加法断言修复任务，展示的内容来自已知受控场景。所有操作仅在预览中改变展示状态，不能执行真实命令。

可以用浏览器直接打开 [四布局对比页面](previews/tui-layouts.html)，通过底部的上一项／下一项或名称选择切换。页面是自包含 HTML，不需要运行 Saki 或使用 API key。

对话内预览包含布局切换、工具展开、右侧详情选择与抽屉收起；配色和近似字符列宽使用设计控件调整。近似列宽只是浏览器中以等宽字符估计的宽度，不是终端 cell 布局实现。

浏览器检查只能覆盖稿件的布局与交互，不构成 TUI 实现验收。实际程序的终端与模型验证见上面的“实际验收”；真实输入法候选窗仍不在已验证范围内（主动取消已在“实际验收”中通过）。

Chrome 已检查四种布局在明／暗外观、约 320／736／1024 像素内容宽度下的 24 个组合，没有横向溢出或 JavaScript 运行错误；选中工具改变详情、抽屉展开／收起均通过。窄屏的双栏稿实际转为上下布局。原始检查记录位于 .saki/verification/tui-layouts/browser-checks.json。
