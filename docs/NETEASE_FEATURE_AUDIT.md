# 网易云功能审计与补齐路线图

更新时间：2026-10-28
最近复核：2026-10-28（按当前工作树核对导航 section、评论/社交、云盘匹配与歌单同步）

## 结论

当前项目正在从“本地播放器 + 网易云只读播放适配”向完整客户端推进。本地播放链路比较完整；网易云搜索已支持四种类型、服务端分页和实体详情跳转，试听、会员和购买限制已传到 UI，歌单写接口与带类型的客户端也已补齐。仍未完成的是榜单分类筛选、云端封面上传、MV 下载、歌曲百科与完整专辑元数据，以及真实账号下的写操作和登录验证。

网易云接口属于非官方协议。所有涉及账号、播放权限和写操作的功能，都必须在真实账号、当前接口版本和合规范围内验证；网络层不能因为接口返回成功就假定 UI 可以安全展示或持久化结果。

## 主要问题

### 1. 搜索实际只有歌曲（已修复；审计初稿结论已过时）

审计初稿依据当时 UI 固定调用 `SearchKind::Songs` 判定“搜索实际只有歌曲”，该结论已过时。`crates/nekonetease-views/src/ui/search/fetch.rs` 现在携带 `SearchRequest { kind, offset }`、`SEARCH_PAGE_SIZE = 30`，以及带服务端总数和前后页标记的 `SearchPageState`；`crates/nekonetease-ui/ui/views/search-view.slint` 提供 Songs/Albums/Artists/Playlists 四个 pill、`1–30 / N` 摘要、Previous/Next 按钮、Top Result 卡，以及可打开专辑/歌手/歌单详情的实体行；`crates/nekonetease-net/src/netease/client/discovery.rs` 发送 `type: kind.code()`、`limit: 30`、`offset` 和 `total: true`。四种类型的排序质量和 Top Result 命中规则仍需真实账号抽查（未验证）。

### 2. “显示全部歌曲”没有真正分页（已修复；审计初稿结论已过时）

该结论同样已过时。分页状态由页面持有：`fetch.rs` 的 `SearchPageState` 使用服务端 `SearchResult` 的 `song_count`/`album_count`/`artist_count`/`playlist_count` 计算总数与 `has_next`，`callbacks/query.rs` 以 offset 发起上一页/下一页请求，`search-view.slint` 已不再用“显示全部”展开本地 30 条。末页、搜索结果总数变化和翻页期间的请求取消仍需真实账号验证（未验证）。

### 3. 云歌单写操作（协议层已补齐；创建/改名/删除已开放）

`endpoints.rs` 已注册，`client/playlist_write.rs` 已有对应带类型方法，`CloudService` 也已暴露应用层方法。"我创建的歌单"（category 1）已开放创建、改名和删除，描述随创建/改名一并写回；收藏歌单仍只读。增删曲目、拖拽排序、标签编辑、封面更换和取消收藏均已接到 UI（`crates/nekonetease-ui/ui/views/my-library/playlists-tab.slint` 的 `editable` 以 category 作为归属判据）。已按 api-enhanced（commit `2db6684`）核对：

- 创建 `/api/playlist/create`（WEAPI，`{name, privacy, type}`）。
- 删除 `/api/playlist/remove`（WEAPI，`{ids:"[1,2]"}`）。
- 重命名/描述/标签 `/api/playlist/update/name`、`/api/playlist/desc/update`、`/api/playlist/tags/update`（EAPI）。
- 增删/排序 `/api/playlist/manipulate/tracks`（EAPI，`{op:"add"|"del"|"update", pid, trackIds:"[1,2]", imme:true}`）。
- 取消收藏 `/api/playlist/unsubscribe`（EAPI）；收藏仍为 `/api/playlist/subscribe`。

写操作接入 UI 前必须在真实账号上验证权限、失败回滚和请求代次。当前“我创建的歌单”已开放创建/改名/删除、曲目增删、标签编辑和排序；非自己创建的歌单只读，但可收藏/取消收藏。封面编辑支持选择本地图片生成本地封面，云端封面上传仍未实现。线上权限、失败回滚与请求代次均未验证。

### 4. 排行榜和公开发现（部分完成）

搜索页空态已接入热搜、推荐歌单与排行榜：`crates/nekonetease-views/src/ui/search/discover.rs` 调用 `state.cloud.hot_search()`、`discover()` 与 `charts()`，`crates/nekonetease-ui/ui/views/search/discover-section.slint` 渲染热搜 chip、推荐歌单行和榜单行。该改动已在本工作树完成，搜索 UI 的定向测试覆盖了模型、请求与请求代次保护；真实账号下的排序质量和推荐内容仍未验证，不计为已发布。

`/api/toplist` 返回的榜单作为播放列表行展示，点击即打开现有云歌单详情，其曲目列表就是榜单详情与播放入口。榜单页已作为独立 section（导航 12）落地；仍缺榜单分类筛选。

### 5. 播放权限（试听/VIP/购买已传达；地区限制未实现）

`nekonetease_core::entities::cloud::PlaybackRestriction` 定义 `Trial { seconds }`、`VipRequired`、`AlbumPurchaseRequired`；`client/playback.rs::trial_restriction` 从 `freeTrialInfo` 解码可播放的试听窗口（`fee_verdict` 只在没有 URL 时把 `fee` 1/4 映射为会员/购买拒绝），挂在 `StreamTicket.restriction` 上，`CloudPlaybackResult.restriction` 带到 UI，并触发 `TrialPlayback`/`VipRequired`/`AlbumPurchaseRequired` toast（`nekonetease-core::utils::toast`）。地区限制没有实现，也没有对应的错误分支或文案。实际服务音质已回传 UI：`StreamTicket.level` 保留服务端实际返回的档位，起播时写入 `CloudPlaybackResult.served_level`，同时随 `StreamInfo` 挂到播放中的 `TrackSummary.stream` 上，Now Playing 的技术信息芯片行据此显示流本身的 `format` 与按 `size / duration` 实测的平均码率（`ui::now_playing::metadata::to_slint_stream_meta`）。云曲目的芯片不再查询本地 `tracks` 表——网易云 id 与本地 id 不是同一个空间，同号的本地行曾经可能冒充它。

### 6. 网易云数据模型（部分补齐）

`Song` 已补 `alias`、`translated_names`、`mv`、`pop`；`Playlist` 已补 `creator`、`tags`、`play_count`、`subscribed`、`subscribed_count`；`Account` 已解码 `vip_type` 与 `level`（`crates/nekonetease-core/src/entities/cloud.rs`）；`TrackSummary.stream: Option<StreamInfo>` 携带流实际服务的 `format`、`level` 与 `size_bytes`，本地行恒为 `None`，并且不映射任何数据库列（`#[sqlx(skip)]`），所以投影查询不会为它付一次解码。可用音质与实际降级结果、云贝/签到、评论分页与点赞、MV/视频元数据均已完成；仍缺专辑发行信息与歌曲百科。`Album` 目前仍只有 id、名称、封面和艺人。

### 7. 媒体类型仍然是纯音频

数据库仍只承载本地音频（基线写明“no video support”，`migrations/20260514000000_initial_schema.sql:1`），MV 不落库，按远端实体读取。已完成 MV 浏览与播放：`Mv`/`MvStream` 实体、`/api/v1/mv/detail`、`/api/song/enhance/play/mv/url`（`r:1080`）、`/api/artist/mvs`（服务端分页）、`/api/discovery/simiMV`，云歌手页有 MV 卡片条，MV 详情页给出封面/标题/艺人/日期/时长/播放量/简介。播放走独立的 `nekonetease-video`（libmpv 2.5，软件渲染成帧 → Slint `Image` 覆盖层，硬件解码默认 `hwdec=auto-safe`，设置页可关）。播客/DJ 已完成；MV 下载尚未落地。

### 8. 账号功能（部分）

当前支持二维码、手机号、邮箱登录和退出，另有云贝余额与每日签到（`/api/v1/user/info` 的 `userPoint.balance` 与 `mobileSign`/`pcSign`，`/api/pointmall/user/sign`，均在设置页，成功即禁用按钮，代次守卫防止旧余额读回把按钮重新点亮）。仍缺验证码登录、登录风控处理、密码重置、设备管理与登录会话、账号安全信息。`Account` 已解码 `vipType` 与 `level` 并展示在昵称下，头像也已绘制；真实账号登录探针仍未执行（未验证）。签到是否按客户端分开计算（`mobileSign` 与 `pcSign` 的语义）没有双方文档可依，当前取两者的 OR，需真实账号确认。

### 9. 社交功能（评论、用户主页与消息）

评论已按资源线程统一：`CommentThread::{Song,Album,Playlist}` 映射 `R_SO_4_` / `R_AL_3_` / `A_PL_0_` 前缀，云歌单详情与云专辑详情都预览最近 4 条并带加载/错误/重试；歌曲、歌单与 MV 评论都从同一整页评论区进入。评论点赞已完成（`/api/comment/like` 与 `/api/comment/unlike`，乐观更新、失败只在行仍是乐观态时回滚、旧响应按请求代次丢弃），评论 id 用字符串保存——Slint 的 `int` 是 32 位，网易云评论 id 会超界并塌成同一个值。回复、发表、热门排序、用户主页、关注、动态、私信与通知均已实现且只读不落库；分享已支持复制官方链接。

### 10. 接口协议漂移（路径漂移已解决；字段形状已容错）

旧审计记录文档写 `/api/song/enhance/player/url/v1` 而代码注册 `/api/song/enhance/player/url`。代码现在同时注册 `STREAM_V1`（XEAPI，当前路由）和 `STREAM`（EAPI，回退），`docs/netease-api.md` 已与代码一致，这处漂移已解决。非官方接口的路径、字段和加密要求仍可能变化，写操作、登录和权限字段必须用真实账号持续验证。

字段形状同样会漂移，而且比路径更隐蔽：真实响应里 `playCount` 是 JSON 浮点（`25370443800.0`）、`subscribed` 对未登录请求是 `null`、`pop` 也是浮点。这些字段一旦声明成严格的 `u64`/`bool`，serde 拒绝的是**整个响应**，UI 只能看到“无法识别的数据”，整页歌单、榜单、搜索同时失效。现在的解码对计数、分数和布尔标记做了容错（浮点取整、字符串数字、`null` 视为假），并由 `crates/nekonetease-core/src/entities/tests/cloud_tests.rs` 钉住；`decode` 在 debug 级记录 serde 拒绝的原因，下次漂移能直接看到是哪个字段，而不是重新从零排查。

### 11. 线上协议测试不足

密码登录和二维码登录仍需真实账号手工验证（`docs/netease-api.md`）。听歌上报是 best-effort，崩溃、离线退出和队列溢出都会丢失记录。新增的歌单写接口、歌单评论、搜索分页和限制提示同样只有解码/单元测试，没有真实账号探针。

## 功能补齐优先级

### P0：搜索与内容发现

1. 歌曲、专辑、歌手和歌单四种搜索模式——已完成（网络层 + UI）。
2. 搜索建议、热搜和综合匹配——已完成搜索建议（`/api/search/suggest/web`，150 ms 尾随防抖 + 请求令牌，弹层在输入框下方）与空态热搜；“综合匹配”仍按四类分别请求，没有独立的综合结果排序。
3. 服务端总数、分页和翻页——已完成；无限滚动未实现。
4. 推荐歌单、每日推荐、私人 FM、排行榜和榜单详情——已完成。榜单走 `/api/toplist/detail`（WEAPI，空 body 一次返回全量目录，线上探针 65 个）；返回里**没有分类字段**，只有每榜的 `tags`，所以做成服务端顺序的全量列表、tags 作副标题，不臆造分类；点开榜单复用现有云歌单详情，播放/评论/收藏直接可用。已知限制：未登录时打开榜单会失败并停在歌单网格（缺公开歌单回退），详情返回箭头回歌单页而非搜索页。
5. 搜索结果中的专辑、歌手和歌单直接打开云详情——已完成。歌手这条此前是错的：Top Result 的艺人走的是**本地**歌手详情，拿网易云 id 查本地表必然 `Artist not found`；现在经 `cloud_tracks::open_entity_detail` 打开云端歌手详情。

搜索侧的 `SearchRequest`、服务端总数和实体导航已经落地；下一步是把同一分页模型复用到评论、推荐和榜单，并由页面持有分页状态。

### P0：云歌单完整管理

1. 创建、重命名和删除歌单——已完成，限于“我创建的歌单”；描述随创建/改名写回。
2. 添加、删除歌曲——已完成：云行右键菜单可“加入歌单”（自己创建的歌单）或“从此歌单移除”（仅自己创建的歌单）。
3. 拖拽调整歌曲顺序——已完成：仅自己创建的歌单可用，乐观重排 + 失败回滚 + 成功后刷新；带完整性守卫——详情持有行数与服务端 `trackCount` 不一致时拒绝写回，避免把批次漏掉的歌曲从云歌单里删掉。
4. 编辑歌单描述和标签——已完成：描述随创建/改名写回，标签通过“Tags”对话框编辑（逗号分隔）；封面编辑未接 UI（当前协议没有已核实的封面上传路径）。
5. 收藏/取消收藏后的即时同步和失败回滚——已完成：云歌单详情对非自己创建的歌单显示“收藏/已收藏”，写入成功后才更新按钮，失败弹 toast 且保持原状态。

写操作应集中在 `CloudService`，由网络客户端提供类型化方法；视图只提交意图并消费结果。歌单编辑需要请求代次、取消令牌和乐观更新回滚，避免快速切换账号或页面时把旧结果写入新歌单。

### P1：播放和账号状态

1. 显示无版权、会员、试听、购买和地区限制原因——已完成其中四类：试听、会员、购买，以及 `Unavailable { NoCopyright | NotAvailable | Unknown }`（详情带 `noCopyrightRcmd`、`privilege.st < 0`、`pl==0 && dl==0`）。**地区限制在线上不可区分**——按 IP 判定、与其它拒绝同样是空 URL，详情常常仍给播放权，因此归入 `Unknown` 并写明理由，而不是猜一个字段。
2. 展示歌曲可用音质与实际降级结果——已完成：实际服务音质低于请求档位时起播弹一次降级 toast，`CloudPlaybackResult.served_level` 保留结果，播放中的 `TrackSummary.stream` 把档位、容器与实测码率送到 Now Playing 芯片行；按曲目列出可选音质仍未做。
3. 展示 VIP、等级、云贝和签到状态——已完成：设置页展示 VIP 与等级，云贝余额与每日签到按钮（成功即禁用，代次守卫防止旧余额读回重新点亮）。
4. 展示账号头像、设备和登录会话——已完成：头像（generation 防串号）、验证码登录、风控错误码映射、登录设备列表与注销。
5. 稳定播放票据刷新和离线重试队列——票据带过期时间并按缓存校验；听歌上报已落盘到 `listen-queue.json`（上限 200 条、7 天过期、按账号隔离），启动/登录/定时重试，仍会丢失采样未覆盖与硬崩溃窗口内的记录。

播放错误需要使用稳定的领域枚举传到 UI，再由翻译层生成文案；不要让 UI 解析网络错误字符串。票据刷新必须绑定账号 generation，离线重试队列必须有上限、过期时间和退出时的持久化策略。

### P1：网易云内容类型

1. MV 详情、MV 播放和相关视频——已完成详情、播放（libmpv）与相似 MV 卡片条。
2. 播客/DJ 电台、节目详情和订阅——已完成：电台推荐条、电台详情与订阅、节目列表（30 条分页）与节目详情，播放走现有云播放链路（节目可播身份是 `mainSong.id`）。`/api/dj/program/up` 无独立来源佐证，未实现。
3. 歌曲百科、版本信息和专辑完整元数据——`Song.alias`/`translated_names` 已解码，其余未实现。
4. 视频封面与视频缓存策略——封面沿用云实体封面；视频本身不缓存，每次播放重新协商 URL。

视频和音频应使用不同的媒体实体与缓存策略，不要把视频 URL 塞进 `Song` 或本地音轨表。新增媒体类型前应先确定播放器能力边界和数据库迁移方案。

### P2：社交

1. 歌曲、歌单、专辑和 MV 评论——整页评论区已实现（时间序/热门序、分页、回复、发表、删除、点赞），从云专辑详情、云歌单预览、Now Playing 与 MV 详情进入。
2. 评论点赞、回复和热门评论——已完成：点赞（乐观更新 + 条件回滚）、回复（floor 单页）、热门/最新切换、发表与删除。评论 id 一律走字符串，避免 Slint 32 位整型把大 id 塌成一个值。
3. 用户主页、关注、动态、私信和通知——已完成：主页（头像/关注数/歌单/动态）、关注与取关（乐观 + 回滚）、私信会话与消息、通知与评论通知；全部只读且**不落库**，模块内不打日志，入口在设置页账号区。
4. 分享歌曲、歌单、专辑和播放列表——已实现复制官方分享链接（`https://music.163.com/{song,album,artist,playlist}?id=`），通过现有剪贴板通道；尚无系统分享面板。

社交功能涉及大量分页、权限和敏感数据，应先建立统一的远端分页模型与请求取消策略，再逐项接 UI；私信和通知不能在没有本地隐私设置与清理策略前直接落库。

### P2：云盘与离线

1. 网易云云盘歌曲列表——已完成：`/api/v1/cloud/get`（WEAPI，只读，`{limit, offset}`，条目在 `data[].simpleSong`、总数在 `count`），“收藏”页签新增“云盘”页，50 首一页带“更多”，分页缓存按账号代次隔离并做去重合并。
2. 云盘歌曲与本地歌曲匹配——后端已完成（标题 NFKC 归一 + 艺人集合解析 + 3 秒时长容差，宁可漏配不乱配），已接入“收藏 ▸ 云盘”页。
3. 合法范围内的离线缓存和缓存清理——已完成并接入播放：按「歌曲 id + 实际服务档位」落盘，2 GiB 上限、单曲 1/4 上限、30 天按最后使用过期，6 小时清扫；**试听流永不缓存**，命中即本地解码、未命中照常流播并在起播后后台缓存。
4. 云端歌单与本地歌单同步及冲突处理——后端已完成：三方差异（含上次同步快照）、先出 dry-run 再应用、默认本地优先；`SyncPlan` 无公开构造函数，所以“不先看差异就应用”在类型上不可表达。已接入歌单详情的复核面板。

离线功能必须明确缓存加密、磁盘占用上限、版权限制和清理行为。云端歌单同步应使用独立的同步记录和冲突策略，不能直接覆盖本地歌单。

## 社区接口参考

社区网易云 API 通常会把上述能力映射到以下路径族，但这些路径并不稳定，不能只凭路径名实现：

| 能力 | 常见路径族 |
| --- | --- |
| 评论 | `/api/v1/resource/comments/A_PL_0_{id}`（歌单，已验证）；`comment/*`（其他资源族） |
| MV/视频 | `mv/*`、`video/*` |
| DJ/播客 | `dj/*` |
| 歌单管理 | `playlist/create`、`playlist/remove`、`playlist/update/name`、`playlist/desc/update`、`playlist/tags/update`、`playlist/manipulate/tracks`、`playlist/subscribe`、`playlist/unsubscribe` |
| 用户信息 | `user/detail`、`user/follows`、`event` |
| 云盘 | `cloud` |
| VIP | `vip/info` |

实施前必须使用当前客户端协议和真实账号验证请求体、加密方式、字段含义、分页规则及权限错误；同时遵守网易云服务条款和相关内容授权范围。

## 当前状态与下一步

审计列出的 P0/P1/P2 主干已在本工作树落地：搜索四类型 + 分页 + 建议 + 热搜/推荐/全量榜单、云歌单 CRUD + 增删曲目 + 拖拽排序 + 标签/描述、播放拒绝分类（试听/会员/购买/无版权/不可用）、音质回退与芯片行、VIP/等级/云贝/签到/设备会话/验证码登录、MV 详情与播放（libmpv）、播客/DJ、评论整页（分页/回复/发表/删除/点赞/热门）、社交（主页/关注/动态/私信/通知）、云盘列表、离线流媒体缓存（已接播放）、云盘↔本地匹配与云本地歌单同步（均已接入 UI）。

**有意的取舍**：地区限制线上不可区分，归入 `Unknown`；榜单没有分类字段，按服务端顺序展示并列出 tags；「综合匹配」没有跨类型相关度字段，保持四类分页；云端封面上传需要一条 NOS 二进制上传原语，未实现。

**仍缺**：MV 下载；歌曲百科/版本信息；完整专辑发行信息；榜单分类筛选与未登录时的公开歌单回退。

每个阶段都应同时增加网络解码测试、应用服务测试、UI 状态测试和一次真实账号手工探针；没有线上验证的接口只能标记为实验能力，不能在 README 中宣称完整支持。
