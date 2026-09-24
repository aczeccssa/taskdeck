# Taskdeck 文档

Taskdeck 是项目任务的持久控制平面。请先阅读[安装指南](../../#install)，然后配置 worker 或 leader，并在 9837 端口打开 Web UI。

文档内容与 Release 一起维护，因此每个 Git tag 都可以发布对应版本的文档快照。

## 当前版本：v0.2.0

本版限制审计记录增长和搜索索引范围；原生安装的远程监听必须显式授权；已是当前 schema 的数据库不再争抢迁移锁或在每次打开时执行完整性扫描；定时任务的跳过、停止和重启收尾历史也更完整。

查看 [v0.2.0 Release](https://github.com/aczeccssa/taskdeck/releases/tag/v0.2.0)、[变更记录](https://github.com/aczeccssa/taskdeck/blob/master/CHANGELOG.md) 或[对应版本文档](https://aczeccssa.github.io/taskdeck/versions/v0.2.0/)。SQLite 空间回收步骤及已知限制均列在变更记录中。
