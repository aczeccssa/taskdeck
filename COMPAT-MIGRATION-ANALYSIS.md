# Taskdeck 用户配置兼容迁移实施规范

> 生成时间：2026-09-10
>
> 代码基线：`5a76f11`（分支 `workbuddy/refactor-src-module-split-20260909`）
>
> 工作区：`/Users/a/Workbuddy/Worktrees/taskdeck/codex-ui-visual-audit-20260907-dbcc2e92`
>
> 目标配置：默认路径 `~/.taskdeck/taskdeck.json`；实现统一使用 `$TASKDECK_HOME/taskdeck.json`
>
> 本文是实施与验收规范；修改代码前后必须逐项记录 checkpoint 证据。

---

## 0. 结论与已决策语义

Taskdeck 新增用户级配置文件不是一次“全新安装专用”的变化，而是一项正式的**向后兼容迁移**：

1. 已有有效 `taskdeck.json`：直接使用，不覆盖、不猜测、不重写用户值。
2. 旧安装没有 `taskdeck.json`：自动读取旧数据库中的有效配置，原值生成 `taskdeck.json`，保持升级前的监听行为。
3. 全新安装没有旧数据库配置：自动生成 `taskdeck.json`，使用安全默认值 `127.0.0.1:9837`。
4. 新配置原子落盘、回读校验和生效验证全部成功后，主动清理数据库中的废弃配置字段。
5. 清理废弃字段属于兼容迁移的 contract 阶段，不是无条件删除；必须先备份、可重试、可观测，并提供回滚路径。
6. 无效或未来版本的 `taskdeck.json` 必须明确报错，禁止静默回退到数据库、`0.0.0.0` 或其他默认值。

因此，旧安装不会因为“缺少新文件”而启动失败，也不会把旧用户主动选择的 `127.0.0.1` 或自定义监听地址擅自改成另一个值。旧的 `taskdeck.json` 若已有远程监听但缺少 `allow_remote_bind`，会 fail closed；管理员必须执行 `taskdeck node configure --allow-remote-bind` 完成一次明确确认。无 `taskdeck.json` 的旧安装仍会从数据库迁移原值，并由该命令生成带显式字段的新配置。

---

## 1. 当前实现的已确认问题

### 1.1 数据库迁移没有形成安全边界

- `metadata.schema_version` 当前为 8，但 `PRAGMA user_version` 从未设置。
- v1–v7 会一跳升级到 v8，没有独立的 `v_n -> v_n+1` 迁移函数。
- 迁移前没有一致性备份，失败后没有正式恢复入口。
- `CREATE TABLE IF NOT EXISTS` 会把缺失表静默重建为空表，掩盖损坏或数据丢失。
- `bind_host=127.0.0.1` 会在旧库升级时被静默改写成 `0.0.0.0`。

### 1.2 高版本拒绝发生得太晚

当前 `StateStore::open` 在检查 `schema_version` 前已经：

1. 设置 `journal_mode=WAL`；
2. 执行整套 `CREATE TABLE IF NOT EXISTS`；
3. 补 `registrations.alias` 和 `workflow_groups.graph_json`；
4. 最后才拒绝未来版本。

临时 v9 数据库实测：命令最终报 `unsupported state database schema version '9'`，但数据库表数量已经从 1 增加到 22。因此“拒绝未来版本”必须提前到任何写操作之前。

### 1.3 网络配置目前由数据库持久化

`bind_host` 与 `web_port` 当前和节点角色、名称、leader 配置、enrollment token 一起存放在 metadata，并通过同一个数据库事务写入。迁移到 JSON 后必须覆盖 CLI、IPC、Web API 和集群远程配置等全部读写入口，不能只修改 `taskdeck node configure`。

### 1.4 平台兼容问题

- systemd unit 把 POSIX shell 单引号嵌入 `Environment=`，daemon 实际收到带单引号的 `TASKDECK_HOME`。
- 官方 Debian 运行镜像缺少 `lsof`，监听端点探测静默降级。
- Windows 目标当前存在重复函数定义，不能完成 Windows 迁移验收。
- IPC 没有显式版本握手；agent 协议已有版本校验。

---

## 2. 配置格式与权威来源

### 2.1 配置文件

最小合法配置：

```json
{
  "version": 1,
  "bind_host": "127.0.0.1",
  "web_port": 9837
}
```

约束：

- 默认位置是 `~/.taskdeck/taskdeck.json`。
- 设置 `TASKDECK_HOME` 时，位置是 `$TASKDECK_HOME/taskdeck.json`。
- `version` 是用户配置格式版本，不等同于数据库 schema 版本。
- `bind_host` 不得为空；`web_port` 必须处于 `1..=65535`。
- 未知未来版本必须拒绝；v1 中的未知字段先告警并保留，不能在自动写回时丢失。
- 原子写入必须使用同目录临时文件、flush、必要的 fsync、rename 和回读解析验证。
- Unix 下创建文件应使用仅当前用户可读写的权限；不得跟随不受信任的符号链接覆盖任意文件。

### 2.2 运行时优先级

优先级固定为：

```text
显式 daemon CLI 参数
  > TASKDECK_BIND_HOST / TASKDECK_WEB_PORT 环境变量
  > $TASKDECK_HOME/taskdeck.json
  > 全新安装安全默认值 127.0.0.1:9837
```

现有 `taskdeck daemon --web-port` 属于显式 CLI 参数，必须保留最高优先级，或在独立兼容周期内正式废弃；不得处于未定义状态。

环境变量是部署时覆盖，不得自动写回 JSON。Compose / Docker 继续显式设置 `TASKDECK_BIND_HOST=0.0.0.0` 并配套 `TASKDECK_ALLOW_REMOTE_BIND=true`，从而保持容器对外监听行为。

### 2.3 旧安装缺少配置文件时的迁移矩阵

| 场景 | 生成内容 | 是否清理旧字段 | 结果 |
| --- | --- | --- | --- |
| 已有有效 JSON | 保留原文件 | 完成权威切换后清理 | JSON 权威 |
| 无 JSON，旧 metadata 有合法 `bind_host` / `web_port` | 原值复制到 JSON | 验证成功后清理 | 保持升级前行为 |
| 无 JSON，旧 metadata 仅缺一个字段 | 合法旧值 + 缺失字段安全默认 | 验证成功后清理 | 明确告警并可继续 |
| 无 JSON，旧 metadata 值非法 | 不生成猜测配置 | 不清理 | 报可操作错误 |
| 无 JSON、无旧数据库或明确为空的新库 | `127.0.0.1:9837` | 无旧字段可清理 | 全新安全默认 |
| JSON 无效或版本高于当前 | 不覆盖 | 不清理 | 拒绝启动并报告路径 |
| JSON 与旧 metadata 不同 | 使用 JSON | 记录差异后清理 metadata | 尊重用户新配置 |

迁移读取的是数据库中的**原始持久值**，不能把当前进程的环境变量覆盖结果误写进 JSON。

---

## 3. 数据库版本和兼容窗口

### 3.1 版本识别必须先于修改

对于已经存在的 `state.db`，首先使用只读连接探测：

- 文件是否为有效 SQLite 数据库；
- `PRAGMA user_version`；
- metadata 表是否存在；
- 旧 `metadata.schema_version` 的值。

在完成版本判定前禁止设置 WAL、建表、补列或写 metadata。高于当前版本的数据库必须在零修改状态下拒绝，并验证 schema 与数据摘要未变化。

### 3.2 `user_version` 接管矩阵

| `user_version` | metadata 版本 | 判定 |
| --- | --- | --- |
| 0 | 不存在，且数据库为空 | 全新数据库 |
| 0 | 合法旧版本 1..8 | 旧库接管，以 metadata 作为一次性来源 |
| 与 metadata 相等 | 当前支持范围内 | 正常 |
| 非 0 且与 metadata 不同 | 任意 | 拒绝自动迁移，进入诊断/repair |
| 任一来源高于当前版本 | 任意 | 零修改拒绝启动 |
| 低于最小支持版本 | 任意 | 指示先升级到中间版本 |
| 值非法、metadata 缺失但库非空 | 任意 | 拒绝猜测，进入诊断/repair |

迁移成功后 `PRAGMA user_version` 是权威版本；metadata 版本在兼容窗口内作为旧二进制的拒绝保护同步更新。两者冲突不能只记 warning 后继续写库。

### 3.3 下一 schema 版本

用户配置权威切换和废弃 metadata 清理必须占用一个明确的 schema 版本，例如 `v8 -> v9`。若实现时 v9 已被其他迁移占用，则使用下一个版本，不得把清理行为隐藏在仍标记为 v8 的数据库中。

兼容窗口采用三阶段：

1. **Expand**：新二进制优先读 JSON，但 JSON 缺失时仍可读旧 metadata；暂不删除旧字段。
2. **Migrate**：自动生成并验证 JSON；所有写入口改写 JSON。
3. **Contract**：确认 JSON 已生效后升级 schema，清理 `metadata.bind_host` / `metadata.web_port`，旧二进制依靠高版本门禁拒绝打开。

---

## 4. 迁移执行与失败恢复

### 4.1 执行流程

```text
acquire migration lock
  -> read-only probe database and config
  -> reject future/corrupt state before mutation
  -> create verified SQLite-consistent backup
  -> expand database schema in one transaction
  -> resolve legacy config source
  -> atomically create or validate taskdeck.json
  -> read back and validate effective settings
  -> switch all readers/writers to JSON authority
  -> verify daemon actually binds expected address
  -> contract transaction: bump schema and delete deprecated metadata
  -> record durable migration result
  -> release migration lock
```

### 4.2 锁与备份

- 使用 `$TASKDECK_HOME` 范围的跨进程独占迁移锁。
- daemon 正在运行时，不允许另一个 CLI 进程直接执行清理；应请求 daemon 协调迁移或明确停止并重启。
- 备份使用 SQLite backup API 或 `VACUUM INTO` 等一致性快照机制；不能只在 checkpoint 后裸复制打开中的 `state.db`。
- 备份文件包含来源版本和时间戳，并在迁移前执行完整性检查。
- 单条数据库迁移在一个事务内完成；失败以事务回滚为主，不自动用备份覆盖仍打开的数据库。
- restore 是显式操作：关闭连接、验证备份、保留失败现场后才能恢复。

### 4.3 幂等与崩溃恢复

- JSON 已落盘但 metadata 尚未清理：重试时验证 JSON 后继续 contract，不得覆盖 JSON。
- metadata 已清理但 schema 未提交：数据库事务必须整体回滚，不能留下半清理状态。
- schema 已升级但迁移记录尚未写入：启动时根据 schema、JSON 和迁移标记补记结果，不能重复改写配置。
- 混合更新同时包含 JSON 字段和数据库字段时，必须有可恢复的提交顺序和失败注入测试；禁止返回普通成功却只保存一半。
- 所有步骤可安全重复执行；重复启动不会重写用户配置或生成无限备份。

### 4.4 回滚边界

- Contract 前：旧 metadata 仍在，允许回滚到旧二进制。
- Contract 后：旧 metadata 已清理、schema 已提升；旧二进制必须拒绝打开。
- Contract 后回滚必须恢复迁移前数据库备份；已生成的 JSON 可以保留，但旧二进制不会读取它。
- 自动迁移不得删除项目级 `taskdeck.yaml`、`.vscode/tasks.json`、用户任务或其他业务数据。

---

## 5. 读写入口改造范围

以下入口必须统一使用同一个配置解析器、优先级解析器和写入器：

- daemon 启动及 `configured_settings`；
- `taskdeck node show`；
- `taskdeck node configure --bind-host/--web-port`；
- IPC `GetNodeSettings` / `PutNodeSettings`；
- Web 节点配置 API；
- leader 对 worker 的远程节点配置；
- service install/status 读取的 `TASKDECK_HOME`；
- Compose、Docker 和测试 fixture。

节点身份、角色、leader 设置和 enrollment token 暂时仍由数据库管理；`bind_host` 与 `web_port` 由 JSON 管理。公开的 `NodeSettings` 视图必须合并两处持久状态及环境变量覆盖，并明确标记每个字段的实际来源。

---

## 6. 实施批次

| 批次 | 内容 | 完成条件 |
| --- | --- | --- |
| 0 | 基线、fixture、零修改未来版本门禁 | CP-0、CP-1 |
| 1 | 数据库迁移地基：锁、备份、事务、版本接管 | CP-2 |
| 2 | Expand：JSON 模型、读取优先级、旧 metadata fallback | CP-3 |
| 3 | Migrate：缺文件自动生成、保留旧配置行为 | CP-4 |
| 4 | 全入口权威切换与混合写恢复 | CP-5 |
| 5 | Contract：提升 schema、清理废弃 metadata | CP-6 |
| 6 | systemd 存量修复与平台验证 | CP-7 |
| 7 | CI、跨版本、跨平台和协议演进 | CP-8 |

每个批次满足对应 checkpoint 前，不得进入依赖它的下一批次。尤其禁止在 CP-4/CP-5 未通过时提前清理数据库旧字段。

---

## 7. 强制 Checkpoint

> 每项必须记录日期、commit、实际命令、输出摘要、fixture 和剩余风险。没有运行证据时状态保持“未检查”。

### CP-0：工作区与基线保护（状态：未检查）

- [ ] 当前目录确认为 `/Users/a/Workbuddy/Worktrees/taskdeck/codex-ui-visual-audit-20260907-dbcc2e92`。
- [ ] 记录分支、HEAD、`git status --short`；不覆盖现有未跟踪或未提交文件。
- [ ] 记录现有 state/config/platform_service 测试通过数和基线失败。
- [ ] 建立 v1–v8、空库、损坏库、未来版本库 fixture 清单。
- [ ] 确认本迁移不改变 project-level `taskdeck.yaml` 与 `.vscode/tasks.json` 语义。

### CP-1：版本检查零修改（状态：未检查）

- [ ] 未来版本数据库在任何 DDL、补列、WAL 切换之前被拒绝。
- [ ] 拒绝前后比较 `sqlite_master`、关键行数、`user_version`、metadata 和文件摘要，确认零修改。
- [ ] 非 SQLite 文件、损坏 metadata、版本冲突均返回可操作错误，不自动重建。
- [ ] 全新空目录与“已有但空的 SQLite 文件”有明确且经过测试的判定。

### CP-2：迁移地基（状态：未检查）

- [ ] 同一 `TASKDECK_HOME` 只能有一个迁移执行者；并发进程得到明确状态。
- [ ] 使用 SQLite 一致性快照生成备份，并验证可打开和完整性。
- [ ] `user_version=0 + metadata=1..8` 按接管矩阵迁移。
- [ ] 每个 `v_n -> v_n+1` 使用独立迁移函数并在单一事务中执行。
- [ ] 注入迁移失败后，事务回滚、原库可打开、备份可显式恢复。
- [ ] 高版本和低于最小支持版本均给出明确升级路径。

### CP-3：Expand 兼容读取（状态：未检查）

- [ ] 新配置解析、校验、未知字段保留和未来版本拒绝均有单测。
- [ ] 有效 JSON 优先于旧 metadata；JSON 缺失时仍能读取旧 metadata。
- [ ] 环境变量覆盖只影响运行时，不写入 JSON。
- [ ] `taskdeck daemon --web-port`、环境变量、JSON、默认值的优先级逐层验证。
- [ ] 此阶段数据库旧字段仍保留，旧二进制仍可回滚使用。

### CP-4：旧安装自动生成配置（状态：未检查）

- [ ] 无 JSON、旧值为 `127.0.0.1` 时，生成 JSON 后仍监听 `127.0.0.1`。
- [ ] 无 JSON、旧值为 `0.0.0.0` 时，生成 JSON 后仍监听 `0.0.0.0`。
- [ ] 无 JSON、旧值为自定义合法地址/端口时，生成 JSON 后保持原值。
- [ ] 全新安装生成 `127.0.0.1:9837`。
- [ ] JSON 使用同目录临时文件原子写入，权限正确，失败不留下截断文件。
- [ ] 回读 JSON 并实际启动 daemon，确认监听地址与迁移前一致。
- [ ] 无效旧值不被猜测或清理，CLI/daemon 给出配置路径和修复方式。
- [ ] 重复执行不会覆盖 JSON，也不会重复生成无界备份。

### CP-5：配置权威与全部写入口（状态：未检查）

- [ ] CLI、IPC、Web、leader/worker 路径读取同一套有效配置。
- [ ] `node configure --bind-host/--web-port` 原子修改 JSON，重启后生效。
- [ ] `NodeSettings` 返回有效值及环境变量覆盖来源，不暴露过时 metadata 值。
- [ ] JSON 字段与数据库字段的混合 patch 有失败补偿和崩溃恢复测试。
- [ ] 无效 JSON、未来配置版本、无效 host/port 均 fail closed。
- [ ] Docker/Compose 显式 `0.0.0.0` 覆盖仍实际生效。

### CP-6：Contract 与废弃数据清理（状态：未检查）

- [ ] 只有 JSON 原子写入、回读、有效设置和实际监听验证都成功后，才允许清理旧字段。
- [ ] schema 提升到专用新版本，并与 `PRAGMA user_version`、metadata 保护版本同步。
- [ ] `metadata.bind_host`、`metadata.web_port` 在单一事务内删除；其他 metadata 不受影响。
- [ ] 清理前后验证关键表、索引、行数和用户业务数据。
- [ ] 清理失败完整回滚，重试不会覆盖 JSON。
- [ ] 旧二进制在 contract 后明确拒绝新 schema，而不是补表或继续写入。
- [ ] 使用迁移前备份完成一次真实回滚演练。

### CP-7：systemd 与存量 home 修复（状态：未检查）

- [ ] systemd `Environment=` 使用 systemd 语义转义，不嵌入 POSIX shell 单引号。
- [ ] 真实 systemd 容器中 daemon 收到的 `TASKDECK_HOME` 不含额外引号。
- [ ] 正确 home 与错误引号 home 同时有数据时，只检测、备份和报告；未经策略不得覆盖。
- [ ] 修复后 service status、daemon 实际 home、JSON 路径和 state.db 路径一致。
- [ ] 用户配置迁移不会把错误 home 的值误当成正确安装的旧配置。

### CP-8：跨平台、CI 与协议（状态：未检查）

- [ ] Windows target 编译通过，并验证默认用户配置路径、原子替换和监听语义。
- [ ] macOS、Linux 用户服务安装和重启路径实际验证，失败不得报告成功。
- [ ] CI 覆盖新安装、v1–v8 升级、已有 JSON、缺失 JSON、环境变量覆盖、未来版本拒绝、失败回滚和 contract。
- [ ] IPC 新旧版本不匹配产生明确握手错误。
- [ ] agent 协议版本继续独立校验；MCP `protocolVersion` 不与内部 schema 版本混用。
- [ ] 至少进行一次“旧二进制 -> expand 版本 -> migrate/contract 版本”的完整升级演练。

---

## 8. 完成定义

只有同时满足以下条件，才能宣称用户配置兼容迁移完成：

- 所有 CP-0 至 CP-8 项目都有实际证据；
- 旧安装缺少 JSON 时自动生成配置，并保持旧监听行为；
- 全新安装默认只监听 `127.0.0.1`；
- 有效 JSON 永远不会被自动覆盖；
- 废弃 metadata 只在新配置验证成功后清理；
- 失败路径可重试、可诊断、可回滚；
- 未来版本数据库在拒绝前保持零修改；
- CLI、daemon、Web、IPC、cluster、service 与容器使用一致的配置语义；
- 工作区无意外修改，测试结果和剩余风险已记录。
