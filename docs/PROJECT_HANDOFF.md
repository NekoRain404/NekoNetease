# NekoNetease 项目交付与开发交接文档

更新时间：2026-10-29  
交付对象：后续维护与网易云功能开发团队  
项目维护者：NekoRain  
参考项目作者：Kenan Salar（Melodia）

## 1. 交付结论

NekoNetease 是一个 Rust + Slint 的跨平台桌面音乐播放器。当前产品形态是“本地音乐播放器 + 网易云音乐在线播放适配”，本地曲库、音频解码、播放队列、歌词和桌面集成较完整；网易云部分已经覆盖账号、搜索歌曲、云歌单读取、在线播放、歌词、每日推荐、私人漫游/心动模式等基础链路，但还不是完整的网易云桌面客户端。

网易云功能审计和后续路线见 [`docs/NETEASE_FEATURE_AUDIT.md`](NETEASE_FEATURE_AUDIT.md)，协议细节见 [`docs/netease-api.md`](netease-api.md)。网易云接口是非官方协议，涉及登录、版权、会员、写操作和听歌上报的能力必须使用当前协议和真实账号重新验证。

交接时请把 `.cargo/config.toml` 纳入版本库。该文件缺失会使依赖 `env!("NEKONETEASE_REPO_ROOT")` 的 crate 无法编译；它是工作区构建配置的一部分，不是本地临时文件。

## 2. 当前版本与仓库状态

- 当前版本：`0.2.0`（首个预发布候选）
- 默认分支：`main`
- 许可：`AGPL-3.0-or-later`
- 项目作者：NekoRain
- 参考项目：Melodia，作者 Kenan Salar
- 远程仓库：`https://github.com/NekoRain404/NekoNetease`（应用标识统一为
  `com.github.nekorain404.nekonetease`）
- 支持平台：Linux、Windows；当前发布构建仅覆盖 `x86_64`，`aarch64` 不受支持——它不在发布矩阵中，且打包脚本、RPM/AppImage 工具链与产物命名都按 `x86_64` 写死，不能只靠增加矩阵槽完成（见第 7 节发布链记录）
- 当前仓库包含 `.github/workflows/release.yml`（格式化、依赖审计、Slint 编译、Clippy、Linux/Windows 测试与发布矩阵）和 `.github/actions/`；构建、测试和发布约定以该工作流和 `.claude/rules/ci-packaging.md` 为准。

工作树在生成本文档时是干净的，历史是一条按功能拆分的线性提交。接手团队开新分支前请先跑一遍
第 6 节的门禁，确认本地工具链与 CI 一致。

## 3. 工作区配置

### 3.1 Cargo workspace

根目录 [`Cargo.toml`](../Cargo.toml) 是虚拟 workspace：

```toml
[workspace]
resolver = "3"
members = ["crates/*"]
exclude = ["winit"]
```

`winit/` 自带独立 workspace，因此不参与根 workspace 的依赖解析。所有 `crates/*/Cargo.toml` 都继承根 manifest 的版本、edition、MSRV、许可证、仓库地址和依赖版本。

工作区统一配置为：

- Rust edition：2024
- Rust MSRV：1.99
- Cargo resolver：3
- 版本：0.2.0
- `slint` / `slint-build`：1.16.1
- SQLx + SQLite + 内置 migrations
- Tokio 多线程 runtime
- Slint FemtoVG 渲染器
- Symphonia 音频解码、cpal 音频输出、纯 Rust Opus 解码

### 3.2 工具链与质量配置

- [`rust-toolchain.toml`](../rust-toolchain.toml)：跟随宿主 `stable`，并要求 `clippy`、`rustfmt`、`llvm-tools-preview`。
- [`rustfmt.toml`](../rustfmt.toml)：Rust 2024 edition/style edition，100 列宽度约定。
- [`clippy.toml`](../clippy.toml)：MSRV `1.99` 和项目复杂度阈值。
- [`CLAUDE.md`](../CLAUDE.md)：架构边界、构建命令、线程规则和跨 crate 禁止依赖。
- [`.claude/rules/`](../.claude/rules/)：音频、数据库、UI、打包、测试、诊断等子系统规则。

### 3.3 `.cargo/config.toml`

当前配置为：

```toml
[env]
NEKONETEASE_REPO_ROOT = { value = "./", relative = true }
```

Cargo 会以 `.cargo/config.toml` 所在仓库为基准解析该相对值，并将 `NEKONETEASE_REPO_ROOT` 注入编译环境。`nekonetease-testkit` 和部分测试通过 `env!("NEKONETEASE_REPO_ROOT")` 查找测试 corpus、迁移文件和仓库资源。

不要改为机器相关的绝对路径，也不要把这个文件加入 `.gitignore`。如果修改配置，至少运行 `cargo test --locked --workspace`，确认测试 fixture 和 `sqlx::migrate!` 仍能找到仓库根目录。

## 4. crate 拓扑与责任边界

| crate | 责任 |
| --- | --- |
| `nekonetease` | 桌面二进制入口、runtime、窗口启动和关闭流程 |
| `nekonetease-app` | `AppState`、后台任务、设置、库 API、网易云应用服务、更新器 |
| `nekonetease-core` | 领域实体、错误、路径、主题和共享基础类型 |
| `nekonetease-net` | HTTP、网易云协议、加密、网络图片和在线歌词 |
| `nekonetease-store` | SQLite、SQLx migrations、本地扫描、标签读写、FTS |
| `nekonetease-audio` | Symphonia/Opus 解码和音频 source |
| `nekonetease-playback` | DSP、混音器、cpal 输出和平台音频设备 |
| `nekonetease-engine` | 播放状态机、队列、动作执行和播放器 view model |
| `nekonetease-views` | Rust 侧 Slint bridge、页面状态、回调和 view model 转换 |
| `nekonetease-ui` | `.slint` UI、翻译资源和生成的 UI 类型 |
| `nekonetease-integrations` | Linux MPRIS、Windows/macOS 媒体控制等系统集成 |
| `nekonetease-platform` | 日志、崩溃报告、托盘、单实例和平台服务 |
| `nekonetease-artwork` | 封面解码、缩放、内容寻址 artwork store 和缩略图缓存 |
| `nekonetease-testkit` | 测试 fixture、corpus walker 和共享测试辅助工具 |

依赖方向必须保持单向：UI 通过 `nekonetease-app` 的 library API 访问数据库和网络；`nekonetease-views` 不直接依赖 `nekonetease-store` 或 `nekonetease-net`。播放设备和音频 callback 线程不能执行网络、日志或阻塞 I/O。

## 5. 运行时、数据和凭据

应用入口在 `crates/nekonetease/src/main.rs`：解析路径、创建 Tokio runtime、初始化 `AppState`、安装 UI、运行 Slint event loop，再执行关闭和数据刷盘。

`NEKONETEASE_DATA_DIR` 可覆盖默认数据根目录。`Paths::rooted_at()` 会在该目录下管理：

| 路径 | 用途 |
| --- | --- |
| `nekonetease.db` | SQLite 数据库和 migrations |
| `settings.json` | 设置 |
| `views.json` | 页面状态 |
| `queue.json` | 播放队列 |
| `search_history.json` | 搜索历史 |
| `listen-queue.json` | 未送达的听歌上报（上限 200 条、7 天过期、按账号隔离） |
| `artwork/`、`artists/`、`lyrics/` | 媒体缓存 |
| `backups/`、`logs/` | 数据备份和日志 |

网易云 cookie 和设备标识写入操作系统 credential store，密码不会持久化。credential store 不可用时，登录可以只在当前进程有效，UI 应提示用户。测试必须使用临时 `NEKONETEASE_DATA_DIR` 或 `Paths::rooted_at()`，不能污染开发者真实库。

数据库 schema 由 [`migrations/`](../migrations/) 管理，已经包含本地曲库、FTS、广播电台和标签词汇迁移；当前 schema 仍是音频优先，没有 MV/视频实体。

## 6. 本地与网易云功能现状

### 本地播放链路

本地扫描、标签读取/写回、SQLite 曲库、搜索、歌单、队列、歌词、封面缓存、均衡器、ReplayGain、无缝播放、系统媒体按键、托盘和更新器已经有实现及测试。Linux 需要音频、字体、Wayland/XKB、OpenSSL 和 `pkg-config` 开发包；Windows 没有项目专用系统依赖。

### 网易云现状

当前可用链路主要包括二维码/手机号/邮箱登录、账号恢复、歌曲/专辑/歌手/歌单四类搜索（服务端分页与 Top Result）、云歌单读取、歌单评论预览、歌曲详情、专辑/歌手详情、歌词、每日推荐、私人漫游、心动模式、收藏歌曲、最近播放、听歌上报，以及试听/会员/购买限制的 toast 提示。搜索页空态已接入热搜与推荐歌单，但该改动仍在当前工作树中，按开发中能力描述。

主要缺口仍包括：

1. 搜索建议已完成（`/api/search/suggest/web`，防抖 + 请求令牌）；榜单已有独立页面（导航 12），仍缺榜单分类筛选。
2. 云歌单写操作已接 UI：“我创建的歌单”可创建/改名/删除、增删曲目、编辑描述与标签、更换本地封面、收藏/取消收藏；拖拽排序已开放，云端封面上传仍未实现。
3. 播放权限覆盖试听、会员和购买，实际服务音质已随 `TrackSummary.stream` 送到 Now Playing 芯片行；地区限制仍未实现。
4. 歌单、云专辑、歌曲与 MV 评论均已接入整页评论区（热门/最新、分页、回复、发表、删除、点赞）；MV 详情与播放已完成（libmpv）；云盘歌曲列表、云盘↔本地匹配、云本地歌单同步与离线流媒体缓存均已接入 UI。仍缺 MV 下载、歌曲百科与完整专辑元数据。
5. 密码/二维码登录、歌单写接口、评论、搜索分页和听歌上报仍需要真实账号手工验证。

完整问题清单和 P0/P1/P2 路线不要复制到新文档，直接维护 [`NETEASE_FEATURE_AUDIT.md`](NETEASE_FEATURE_AUDIT.md)，避免两个路线表漂移。

## 7. 开发记录

### 已提交记录

历史是线性的，全部提交的作者与提交者都是 `NekoRain <61972435+NekoRain404@users.noreply.github.com>`，
与 GitHub 账号 `NekoRain404` 对应。

| 日期 | 提交 | 内容 |
| --- | --- | --- |
| 2026-10-28 | `2e034f4` | 项目基线：Cargo workspace、Slint UI、播放链、网易云客户端、打包脚本与文档的一次性导入。 |
| 2026-10-29 | `cd385db` | 把搜索页发现区的卡片网格补进 `card_selection` 的 `LONE_ROW_MOUNTS` 基线。 |
| 2026-10-29 | `8c00a7d` | MV 播放页与全屏覆盖层共用音量与清晰度控件；新增静音（含 `M`）、失败重试，切换相似 MV 时停掉上一条流。 |
| 2026-10-29 | `d2f8a23` | 四项审计修复：重复 ID 的查询不再丢行、补全哈希分批提交、M3U 导入上限、更新器冒烟测试的精确比对与 `kill_on_drop`。 |
| 2026-10-29 | `5ecc172` | CI 默认只读令牌；签名预检不再运行在 pull request 上。 |
| 2026-10-29 | `f5eea9a` | 全仓标识迁移到 `NekoRain404`（AppStream id、polkit action、desktop-id、D-Bus 接口、更新器仓库），版本切到 `0.2.0`。 |
| 2026-10-29 | `0e31ec5` | README 重写并接入截图、ADR 41–42、交接与开发文档对齐代码、引入 issue/PR 模板与 `.gitattributes`。 |

### 当前工作树开发记录

### 2026-10-27 全量重命名

产品、crate、二进制、数据目录与环境变量已从 `Netodia` 统一改为 **NekoNetease**：

- crate 名 `netodia-*` → `nekonetease-*`，Rust 路径 `netodia_*` → `nekonetease_*`。
- 二进制 `Netodia` → `NekoNetease`（`cargo run -p nekonetease`）。
- 环境变量 `NETODIA_*` → `NEKONETEASE_*`（含 `.cargo/config.toml` 的 `NEKONETEASE_REPO_ROOT`）。
- 数据目录 `Netodia{,-dev}` → `NekoNetease{,-dev}`，数据库文件 `nekonetease.db`；单实例 socket 名随数据目录变化。
- gettext 域随包名变为 `nekonetease-ui`，模板与 16 份 catalog 已同步改名。
- 图标、桌面项、Metainfo、polkit 策略与打包脚本同步改名。
- `docs/adr/` 保持原记录不改写（已接受的 ADR 不重写），README 顶部加了一行改名说明。

兼容性影响：旧版客户端无法通过 `--version` 的 `Netodia ` 前缀检查升级到新命名的构建；旧数据目录不会被自动迁移。项目尚未发布，按一次性改名处理。


以下内容仍未提交（未形成提交记录）：

- **构建基线**：`.cargo/config.toml` 提供 `NEKONETEASE_REPO_ROOT`（已随基线提交）；缺失时 `nekonetease-testkit` 与仓库根路径走查无法编译。
- **网易云实体模型**：`Song` 增加 `alias`/`translated_names`/`mv`/`pop`；`Playlist` 增加 `creator`/`tags`/`play_count`/`subscribed`/`subscribed_count`；`Account` 解码 `vipType`/`level`；新增 `PlaylistComment`、`PlaybackRestriction`，`StreamTicket` 增加 `level`/`restriction`。设置页在昵称下展示 VIP 与等级，头像已绘制。
- **播放协议**：播放改走 `/api/song/enhance/player/url/v1`（XEAPI，`ids`/`level`/`encodeType=flac`），旧 EAPI 路由保留为回退；试听、会员、购买限制成为类型化数据，并在起播时以 toast 提示；实际服务音质低于请求档位时弹一次降级 toast。地区限制仍未实现。
- **流信息与芯片行**：`TrackSummary.stream: Option<StreamInfo>`（`format`/`level`/`size_bytes`，`#[sqlx(skip)]`）在起播时由 `StreamTicket` 写入，Now Playing 的芯片行据此显示流自身的容器与 `size / duration` 实测平均码率；云曲目不再用网易云 id 查询本地 `tracks` 表（两个 id 空间不同，同号本地行曾可能冒充它）。
- **搜索与发现**：四种搜索类型、服务端分页与 Top Result 已在工作树；搜索页空态新增热搜与推荐歌单（`search/discover.rs`、`search/discover-section.slint`）。
- **云歌单**：网络层 `client/playlist_write.rs` 与应用层 `cloud/playlists.rs` 覆盖创建/删除/改名/描述/标签/增删排序/取消收藏；“我创建的歌单”已开放创建、改名、删除（含描述写回），收藏歌单只读；增删曲目、排序、标签、封面与取消收藏均已接入 UI；从本地曲目创建云歌单时不会复制本地 track id，已在日志中说明。
- **评论**：`CommentThread::{Song,Album,Playlist}` 映射 `R_SO_4_` / `R_AL_3_` / `A_PL_0_` 前缀；云歌单与云专辑详情展示最近 4 条，带加载/错误/重试；整页评论区已接入云专辑、云歌单预览、歌曲（Now Playing 入口）与 MV。
- **翻译**：新增 3 个 toast 标题（试听/会员/购买）与 4 个基线遗漏 msgid（`Playlists`、`Not interested`、`Recent comments`、`Sending…`），已写入模板与全部 16 份 catalog。环境没有 `slint-tr-extractor`，条目为手工补齐，需在有该工具的机器上运行 `scripts/update-translations.sh` 复核。
- **代码结构**：`nekonetease-net::netease::client`、`nekonetease-app::cloud`、`nekonetease-app::cloud_playback` 三个单体文件拆成按职责划分的模块（均小于 500 行，公开路径不变）；`playlists/detail` 的云投影与评论拆到 `detail_cloud.rs`。
- **离线听歌上报**：`listen-queue.json`（上限 200、7 天过期、按账号隔离），启动/登录/定时重试；采样未覆盖与硬崩溃窗口仍可能丢失。
- **release 构建**：`cargo build --release -p nekonetease` 通过（fat LTO，约 20 分钟）。
- **仓库基线补齐**：`.github/workflows/release.yml`、`actions/{cargo-audit,headless-audio,build-manifest,minisign-sign}`、`.cargo/audit.toml` 与 `.claude/rules/`（21 份）已补回并随全量改名更新；线程名超 15 字节的 6 处已缩短。
- **分享**：曲目/专辑/歌手/歌单可复制官方分享链接，复用现有剪贴板与 Copied toast。
- **接口容错**：计数字段按 JSON 浮点/字符串解码、布尔标记接受 `null`，visitor 直读不再建中间 `serde_json::Value`；`Song.name` 可空。起因是真实响应里 `playCount` 为 `25370443800.0`、`subscribed` 为 `null`，严格字段导致整页“无法识别的数据”。`decode` 在 debug 级记录拒绝原因。
- **Now Playing 芯片行**：`TrackSummary.stream: Option<StreamInfo>`（`format`/`level`/`size_bytes`）在起播时由 `StreamTicket` 写入，显示流自身的容器与 `size / duration` 实测码率；云曲目不再用网易云 id 查本地表。
- **搜索建议**：`client/discovery.rs::search_suggestions` + `ui/search/suggest.rs`/`callbacks/suggest.rs` + `views/search/suggestions-popup.slint`。
- **评论点赞**：`comment/like`、`comment/unlike`，乐观更新与条件回滚；评论 id 以字符串保存，避免 Slint 32 位整型把大 id 塌成一个值。
- **云贝与签到**：`/api/v1/user/info`、`/api/pointmall/user/sign`，设置页显示余额与签到按钮。
- **云盘**：`/api/v1/cloud/get`，“收藏”第四个页签，50 首分页 + 更多 + 去重合并。
- **搜索 Top Artist**：改为打开云端歌手详情（原路径拿网易云 id 查本地表，必然 `Artist not found`）。
- **MV**：`Mv`/`MvStream` 实体与四条 MV 接口（detail / url / artist mvs / simiMV），云歌手页 MV 卡片条与 MV 详情页。
- **漫游 / 榜单独立页**：`Roaming`（导航 11，复用 My Library 的 RoamingTab）与 `Charts`（导航 12，复用 Search 的榜单行与打开路径）成为真正的 section，各有 `SectionActiveGate`；`Recommended` 也成为独立页面（13，`views/recommended-view.slint`，每日推荐与推荐歌单同一份模型）。`MAX_NAV_INDEX = 13`，并修掉 `fold_retired_nav_index` 把 10（Podcast）错误折叠到 My Library 的问题。
- **MV 播放器**：可拖动进度条（拖动期间忽略回传）、1080p/720p/480p/240p 切换（同实例、带起点、失败回退）、窗口全屏、控件 2.5 秒淡出、音量与快捷键（Space/←→/↑↓/F/Esc）；渲染线程回收上一帧缓冲、不再每帧整帧清零。
- **导航竞态**：回放与四个详情打开路径引入 `ReplayToken`（task-local 传递，不改变公开签名），异步返回后复检，旧结果丢弃。
- **M3U**：读端不再 trim、`#` 仅扩展指令算注释；写端对会被误读的路径使用 `#NEKONETEASE-PATH:<百分号编码>`，普通路径逐字节不变。
- **账号恢复**：`0/2/4/8/16/32/60` 秒共 7 次重试，耗尽后提示重新登录；扫码入口不再被待恢复状态挡住。
- **安全与健壮性（审计修复）**：API 客户端不跟随重定向（防 `x-music-u` 跨域泄露）；`main()` 用 `args_os()` 避免非 UTF-8 路径 panic；单实例转发按接收端上限分帧；卸载只删指回本安装目录的符号链接；`entities/cloud.rs` 拆出 `entities/tolerant.rs`（602→473 行）。
- **侧栏入口**：`Recommended` / `Roaming` / `Charts` 三行。原先只是切到 My Library 或 Search 的快捷入口，现已成为独立 section（导航 11/12/13），各有自己的页面、`SectionActiveGate` 与取数通路。
- **歌曲 → MV**：`TrackListRow.mv_remote_id`（字符串保 64 位，本地行恒空），云曲目右键新增 **Music video**，直接走既有 `cloud_mvs::open_mv`。
- **评论入口**：云专辑详情、云歌单预览的「All comments」，以及 Now Playing 溢出菜单（仅云曲目）——三处都打开同一个整页评论区。
- **代码体积**：`entities/cloud.rs` 拆出 `entities/tolerant.rs`（浮点计数 / 可空布尔的容错反序列化），从 602 行回落到 473 行。
- **播放拒绝分类**：`PlaybackRestriction::Unavailable { NoCopyright | NotAvailable | Unknown }`，把空 URL 的票据与歌曲详情合并后再给出原因；地区限制线上不可区分，归入 `Unknown` 并写明理由。
- **评论区**：整页评论（时间序/热门、分页、回复、发表、删除、点赞），从云专辑详情、云歌单预览、Now Playing 与 MV 详情进入；`CALLBACK_HOMES` 增加 `comments`。
- **播客/DJ**：电台推荐、电台详情与订阅、节目列表与节目详情，导航 10（侧栏 Podcast，`MAX_NAV_INDEX` 9→10）。
- **社交**：用户主页、关注/取关、动态、私信、通知，只读不落库；入口在设置页账号区。
- **离线与同步**：流媒体缓存已接入播放（试听不缓存、2 GiB、30 天、6 小时清扫）；云盘↔本地匹配在“收藏 ▸ 云盘”页，云本地歌单同步（dry-run + 本地优先）在歌单详情的复核面板。
- **榜单**：`/api/toplist/detail` 全量目录，点开复用云歌单详情；分类字段不存在，故按服务端顺序展示。
- **云歌单拖拽排序**：乐观重排 + 回滚 + 完整性守卫（详情行数 ≠ 服务端 `trackCount` 时拒绝写回）。
- **发布链**：Windows 现在出 MSI（`x86_64.msi`）与改名后的便携 ZIP（`x86_64-windows.zip`），Linux 产物统一进 `dist/`，manifest 版本取 `prepare-release.py` 写入的工作区版本；`sign-preflight` 只在有 Secret 的事件上运行，`quality` 用 `always()` 且显式拒绝 `failure`/`cancelled`；签名 action 把 base64 密钥落盘（600、退出清理）并把口令经管道喂给 minisign。`release: published` 与 tag push 都会启动构建；每个矩阵槽用 `actions/attest-build-provenance` 为产物出具 provenance；publish 在 manifest 扫描 `dist/` 之后、签名 manifest 之前生成排序的 `SHA256SUMS.txt`（覆盖包与它们的 `.minisig`）并随资产上传。**仍未做且已记录**：aarch64 矩阵（见第 2 节）与 Linux 打包环境的系统依赖步骤。
- **视频播放**：新 crate `nekonetease-video`（`libmpv2` 6.0，LGPL-2.1，动态链接系统 libmpv，机器上为 2.5.0 / mpv 0.41）。软件渲染成帧 → Slint `Image` 覆盖层；仅在 mpv 的 render-update 回调触发时渲染，缩放上限 960×540，`watch` 只保留最新帧；硬件解码默认 `hwdec=auto-safe`，设置页可关闭。没有 libmpv 的机器上播放器自动失能，其余功能不受影响。

### 2026-10-29 界面统一与发布

- **推荐页与 My Library 推荐分类统一**：侧栏 Recommended（导航 13）改为「每日推荐 / 私人雷达」两张特殊卡 + 推荐歌单卡片网格，与 My Library 原推荐分类同构；My Library 的推荐分类已删除，持久化的 `music_library_category` 通过 `view_state_version` 一次性迁移（旧 2 → 0，旧 3 → 2），`Playlists.category-count` 变为 3。
- **排行榜改为网格**：目录页从单列行改为虚拟化卡片矩阵（复用 `EntityCard` 与分块辅助），加载/失败/空三态与「点开走歌单详情」不变。
- **列表行对齐**：`DiscoverPlaylistRow` 的标题、副标题与尾部播放键统一到 76px 行的同一中线。
- 随分类删除，`"Recommendations"` 与 `"No recommendations today"` 两个 msgid 已从模板与 16 份 catalog 移除。
- 发布：release 构建通过并已安装到 `~/.local/share/NekoNetease`（桌面项 + 图标 + `~/.local/bin/nekonetease` 软链），启动峰值 RSS 223 MB。

### 2026-10-28 收束清理

- 删除 `crates/nekonetease-ui/ui/components/sort-menu-popup.slint`：导出的 `SortMenuPopup` 已无任何挂载，`nekonetease-testkit` 中提及它的注释同步改写；它独占的 `"Sort"` msgid 已从模板与 16 份 catalog 移除（translations 套件驱动核对）。
- 删除 `assets/screenshots/`（7 张、约 2.9 MB）：README 与文档均未引用，是旧界面的历史截图。
- 保留并注明理由：`docs/plans/` 三份仍被规则或交接文档引用；`scripts/generate-test-library.py` 与 `scripts/measure-linux-footprint.sh` 是无引用的开发工具（生成测试库、测量常驻内存），属于有意保留的工具链；`docs/adr/` 按规则不改写。
- 清理后 `fmt --check`、全工作区 clippy、全量测试与构建再次全绿。

### 验证快照（2026-10-29，本机）

- `cargo fmt --all --check`：通过。
- `cargo clippy --all-targets --locked --workspace -- -D warnings`：通过（无任何 `-A`/`allow`）。
- `cargo test --locked --workspace`：52 个测试二进制、3960 项断言全部通过（含 15 项翻译目录校验）。
- `cargo build --release -p nekonetease`（fat LTO）：通过，产物 128 MB（保留符号）；本机同时有其它构建任务，耗时 56 分 41 秒。
- `./scripts/build-tarball.sh` + 解包后的 `install-linux.sh`：安装到 `~/.local/share/NekoNetease`，`--version` 输出 `NekoNetease 0.2.0`。
- release 二进制启动 10 秒采样：`VmHWM` 223 MB（0.1.5 同口径）。
- `cargo run --example netease_live_probe -p nekonetease-app`：真实账号探针 13/13 通过（account / 两类搜索 / 榜单 / MV 详情 / 专辑评论 / 云盘 / 每日推荐 / 推荐歌单 / 歌手 MV / 相似 MV / DJ 推荐 / 私人漫游），会话从系统凭据恢复。
- 发布前已知未验证项：真实账号下的写操作（歌单增删、评论发表/删除、MV 点赞收藏、封面上传、签到）、Windows 实机与 MSI 安装、GitHub Actions 上的 release/provenance 步骤、屏幕阅读器实际输出。

### 历史快照（2026-10-27）

- `cargo fmt --all --check`：通过。
- `cargo test -p nekonetease-core --lib`：160 项通过。
- `cargo test -p nekonetease-net --lib`：125 项通过。
- `cargo test -p nekonetease-app --lib`：714 项通过（含离线听歌队列）。
- `cargo test -p nekonetease-views --lib`：912 项通过（含搜索、云歌单、评论与头像用例）。
- `cargo test --locked --workspace`：全部库测试与集成测试二进制通过（packaging 10/10、updater_contract 5/5、rules_globs 1/1）。`.github/workflows/release.yml`、`actions/{cargo-audit,headless-audio,build-manifest,minisign-sign}`、`.cargo/audit.toml` 与 `.claude/rules/`（21 份）已补回，重命名后超长的线程名已缩短。
- `cargo build --locked -p nekonetease`：通过。
- `cargo test -p nekonetease --test translations`：921 项通过。
- `cargo build -p nekonetease-views --locked`：通过（Slint 编译）。
- `cargo clippy --all-targets --locked --workspace -- -D warnings`：通过（宿主 stable 1.99.1，整个工作区，含测试目标）。
- 全量改名后重新验证：`core`（160）/ `net`（125）/ `app`（714）/ `views`（912）/ `translations`（921）/ `cli_contract` / 二进制构建全部通过。
- `cargo build --release -p nekonetease`：通过（fat LTO，约 20 分钟）。
- `cargo clippy -p nekonetease-views --all-targets --locked`：通过（无 deny 级 lint；测试里的 `unwrap()` 已清除）。
- **工具链差异**：工具链已改为跟随宿主 stable（本机 1.99.1），MSRV 声明为 1.99；`cargo clippy ... -D warnings` 可直接在本机作为 CI gate 运行。
- 未执行：真实账号协议探针、Windows 构建。
- `scripts/check-icons.py` 未运行：本机缺少 Python `fontTools`。
- 体积债：`nekonetease-testkit/src/lib.rs` 782 行、`equalizer.rs` 770 行、`engine/backend/mod.rs` 726 行、`cover_thumbs.rs` 703 行等未拆；均低于 800 行上限，但高于 600 行的容忍线，留待后续按同样方式拆分。

## 8. 新团队接手步骤

1. 检查版本库是否包含 `.cargo/config.toml`，并确认 `git status --short` 中没有未解释的改动。
2. 安装 stable Rust（最低 1.99）及 `clippy`、`rustfmt`、`llvm-tools-preview`，让 `rust-toolchain.toml` 自动生效。
3. Linux 安装：`libdbus-1-dev libasound2-dev libxkbcommon-dev libfontconfig1-dev libfreetype-dev libwayland-dev libssl-dev pkg-config`。
4. 运行基线验证：

   ```bash
   cargo fmt --all --check
   cargo clippy --all-targets --locked --workspace -- -D warnings
   cargo test --locked --workspace
   cargo build --locked --release -p nekonetease
   ```

5. 启动开发版本：

   ```bash
   cargo run --locked -p nekonetease
   ```

6. 使用隔离数据目录进行手工启动和 UI 检查：

   ```bash
   NEKONETEASE_DATA_DIR="$(mktemp -d)" cargo run --locked -p nekonetease
   ```

7. 网易云协议定向验证：

   ```bash
   cargo test --locked -p nekonetease-net --lib
   cargo test --locked -p nekonetease-app --lib cloud::tests
   cargo run --locked -p nekonetease-net --example netease_probe
   ```

   probe 只输出计数和权限状态，不应输出 cookie、密码或签名播放 URL。密码登录和二维码登录必须用测试账号手工完成，不能把账号凭据写入仓库、日志或测试 fixture。

## 9. Git 与提交约定

- 每个功能以独立提交交付，提交前必须查看 `git diff --check` 和 `git diff --stat`。
- 不要覆盖其他团队尚未审阅的工作树改动；先按功能拆分或建立临时分支。
- UI 改动同时检查 `.slint`、`nekonetease-views` bridge、相关 Rust 测试和翻译资源。
- 网络协议改动同时更新 endpoint registry、解码测试和 `docs/netease-api.md`；协议尚未经过真实账号验证时标记为实验能力。
- 数据库改动必须新增不可变 migration，不得修改已经发布的 migration 文件。
- 任何新增外部依赖都要检查 workspace 集中声明、许可证、平台条件和 release 打包脚本。

## 10. 交付验收清单

- [ ] `.cargo/config.toml` 已纳入版本库并在干净 clone 中生效。
- [ ] stable Rust（≥1.99）、format、clippy、workspace tests 和 release build 均有新鲜命令输出。
- [ ] 当前未提交改动已逐项解释并拆分提交。
- [ ] 本地播放、扫描、数据库迁移和数据目录隔离通过手工检查。
- [ ] 网易云登录、搜索、歌单、播放权限和歌词使用真实测试账号完成探针验证。
- [ ] 网易云协议失败、无版权、会员限制、试听和离线状态有可读 UI 反馈。
- [ ] 文档、README、endpoint 路径和实际代码没有协议或功能声明漂移。
- [ ] 发布前已复核 AGPL 义务、第三方资源许可和网易云服务条款。

