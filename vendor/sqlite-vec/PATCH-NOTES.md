# sqlite-vec vendored 副本说明

本目录是 sqlite-vec 0.1.10-alpha.4 的 vendored 副本，用于绕过 crates.io tarball 缺失
`sqlite-vec-diskann.c` / `sqlite-vec-rescore.c` 的打包 bug。

## 我方修改

将 `sqlite-vec.c` 中对以下两个文件的 `#include` 用条件编译宏包裹，默认关闭，提供 stub：

- `#include "sqlite-vec-diskann.c"` → 用 `#if SQLITE_VEC_ENABLE_DISKANN` 包裹（默认 0，stub）
- `#include "sqlite-vec-rescore.c"` → 用 `#if SQLITE_VEC_ENABLE_RESCORE` 包裹（默认 0，stub）

核心 vec0 KNN 功能（全量扫描，浮点向量，L2 距离）不受影响。

DiskANN（基于图的近似最近邻索引）和 rescore（全精度重排序）均为可选优化，对本项目的
基础向量检索需求不必要。

## 清理路径

当 sqlite-vec 发布包含完整源码文件的稳定版本时，删除本目录并移除 `Cargo.toml` 中的
`[patch.crates-io]` 条目即可切换回官方包。

## 上游信息

- 项目：https://github.com/asg017/sqlite-vec
- 版本：0.1.10-alpha.4
- 许可证：MIT / Apache-2.0（见 LICENSE-MIT 和 LICENSE-APACHE）
- 原作者：Alex Garcia
