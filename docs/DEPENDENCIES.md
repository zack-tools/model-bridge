# 依賴

直接使用且有呼叫的 crate：`serde`、`serde_json`、`toml_edit`、`sha2`、`getrandom`、`fs2`。`indexmap` 只為了把解析器釘在 Rust 1.83 能編的版本，程式沒有呼叫它。

桌面殼另釘但未下載：`tauri` 2.12.5、`tauri-build` 2.7.4。見 `THIRD_PARTY_NOTICES.md`。

## Cargo.lock

| crate | version | sha256 |
| --- | --- | --- |
| `block-buffer` | 0.10.4 | `3078c7629b62d3f0439517fa394996acacc5cbc91c5a20d8c658e77abd503a71` |
| `cfg-if` | 1.0.5 | `4e7648175b45a9a48536d676f68d918270699102aa8dab5496df06904c914600` |
| `cpufeatures` | 0.2.17 | `59ed5838eebb26a2bb2e58f6d5b5316989ae9d08bab10e0e6d103e656d1b0280` |
| `crypto-common` | 0.1.7 | `78c8292055d1c1df0cce5d180393dc8cce0abec0a7102adb6c7b1eef6016d60a` |
| `digest` | 0.10.7 | `9ed9a281f7bc9b7576e61468ba615a66a5c8cfdff42420a70aa82701a3b1e292` |
| `equivalent` | 1.0.3 | `00d174d5400e5e8fd687ad1049e2f578285fa914201b1af7e8b112a4546bd826` |
| `fs2` | 0.4.3 | `9564fc758e15025b46aa6643b1b77d047d1a56a1aea6e01002ac0c7026876213` |
| `generic-array` | 0.14.7 | `85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a` |
| `getrandom` | 0.2.15 | `c4567c8db10ae91089c99af84c68c38da3ec2f087c3f82960bcdbf3656b6f4d7` |
| `hashbrown` | 0.16.1 | `841d1cc9bed7f9236f321df977030373f4a4163ae1a7dbfe1a51a2c1a51d9100` |
| `indexmap` | 2.11.4 | `4b0f83760fb341a774ed326568e19f5a863af4a952def8c39f9ab92fd95b88e5` |
| `itoa` | 1.0.18 | `8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682` |
| `libc` | 0.2.191 | `65cd87c9a6d60536580b316ad985726104730a5151c99845f2490428ef91240a` |
| `memchr` | 2.8.3 | `cf8baf1c55e62ffcace7a9f06f4bd9cd3f0c4beb022d3b367256b91b87513d98` |
| `proc-macro2` | 1.0.107 | `985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9` |
| `quote` | 1.0.47 | `1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001` |
| `ryu` | 1.0.23 | `9774ba4a74de5f7b1c1451ed6cd5285a32eddb5cccb8cc655a4e50009e06477f` |
| `serde` | 1.0.215 | `6513c1ad0b11a9376da888e3e0baa0077f1aed55c17f50e7b2397136129fb88f` |
| `serde_derive` | 1.0.215 | `ad1e866f866923f252f05c889987993144fb74e722403468a4ebd70c3cd756c0` |
| `serde_json` | 1.0.133 | `c7fceb2473b9166b2294ef05efcb65a3db80803f0b03ef86a5fc88a2b85ee377` |
| `sha2` | 0.10.8 | `793db75ad2bcafc3ffa7c68b215fee268f537982cd901d132f89c6343f3a3dc8` |
| `syn` | 2.0.119 | `872831b642d1a07999a962a351ed35b955ea2cfc8f3862091e2a240a84f17297` |
| `toml_datetime` | 0.6.11 | `22cddaf88f4fbc13c51aebbf5f8eceb5c7c5a9da2ac40a13519eb5b0a0e8f11c` |
| `toml_edit` | 0.22.22 | `4ae48d6208a266e853d946088ed816055e556cc6028c5e8e2b84d9fa5dd7c7f5` |
| `typenum` | 1.20.1 | `b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20` |
| `unicode-ident` | 1.0.27 | `a2c754d6c33795a1c324727428e5a7dedb5b06195f9890bdbcba760d3e246563` |
| `version_check` | 0.9.5 | `0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a` |
| `wasi` | 0.11.1+wasi-snapshot-preview1 | `ccf3ec651a847eb01de73ccad15eb7d99f80485de043efb2f370cd654f4ea44b` |
| `winapi` | 0.3.9 | `5c839a674fcd7a98952e593242ea400abe93992746761e38641405d28b00f419` |
| `winapi-i686-pc-windows-gnu` | 0.4.0 | `ac3b87c63620426dd9b991e5ce0329eff545bccbbb34f3be09ff6fb6ab51b7b6` |
| `winapi-x86_64-pc-windows-gnu` | 0.4.0 | `712e227841d057c1ee1cd2fb22fa7e5a5461ae8e48fa2ca79ec42cfc1931183f` |
| `winnow` | 0.6.26 | `1e90edd2ac1aa278a5c4599b1d89cf03074b610800f866d4026dc199d7929a28` |

## CLIProxyAPI v8.0.57 壓縮檔

校驗和檔本身的 sha256 是 `2770a25337ddedaa8f76076a7804da0f494b0b66e90cef79240bca4b4424a80b`。

| 檔案 | sha256 | 角色 |
| --- | --- | --- |
| `CLIProxyAPI_8.0.57_linux_amd64.tar.gz` | `1e7b31c4bdbfeb91947fbb686933c36234ffa84c1beb781c7c020c035763545e` | 此次評估，已核對 |
| `CLIProxyAPI_8.0.57_linux_aarch64.tar.gz` | `62ec089e714af886b152e5eaf9010aaa5feb17b2a964d17b338b03316ed22144` | 有檔，未跑 |
| `CLIProxyAPI_8.0.57_windows_amd64.zip` | `45687b19ee71aad70236daefc31d33a8b9390b8ad1831c2c84acadc981a2964c` | 擬議，未跑 |
| `CLIProxyAPI_8.0.57_windows_aarch64.zip` | `c059bcb52b19ddf393c1fb9135dea1f94472a43d47cd55e7e06f6cd0db9265c8` | 擬議，未跑 |
| `CLIProxyAPI_8.0.57_darwin_aarch64.tar.gz` | `dd620c0d24e000ffe908d0d8eeb4a6c7315a31d3776fa09f198856b09fd7821f` | 擬議，未跑 |
| `CLIProxyAPI_8.0.57_darwin_amd64.tar.gz` | `428ad123451238972d208ba76d360df461517647e78bcb2ef8ab7cce5b3eebe1` | 擬議，未跑 |

no-plugin 與 FreeBSD 包在 `checksums.txt`，這次沒有選用。解出的 `cli-proxy-api` 觀察到的 sha256 是 `047e384525c93abe1c98cdf3769ed5b720f621030f9777254c2985c11904c985`。上游只對壓縮檔給校驗和，這個內層雜湊是此次觀察，不是上游發布值。
