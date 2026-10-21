# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.19.5](https://github.com/batleforc/weebo-authentik/compare/af227684407ea8676135a5102fca9c4c37926622..v0.19.5) - 2026-10-21
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.22.1 (#28) - ([af22768](https://github.com/batleforc/weebo-authentik/commit/af227684407ea8676135a5102fca9c4c37926622)) - Maxime, Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context)

- - -

## [v0.19.4](https://github.com/batleforc/weebo-authentik/compare/3de4c60b88fd1672b80f64e96fc6eacd14be3070..v0.19.4) - 2026-10-19
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.21.0, kit's without_selector instead of unselected (#27) - ([3de4c60](https://github.com/batleforc/weebo-authentik/commit/3de4c60b88fd1672b80f64e96fc6eacd14be3070)) - Maxime, Claude Opus 5.5 (1M context)

- - -

## [v0.19.3](https://github.com/batleforc/weebo-authentik/compare/6498176e01b128d02ab6e00f2a1a3efb02401b55..v0.19.3) - 2026-10-19
#### Refactoring
- (**controller**) one shared watch per kind (kit v0.20.0) (#26) - ([dbd0cdf](https://github.com/batleforc/weebo-authentik/commit/dbd0cdfbcf52520b1534e9f701c0f3039d8189bd)) - Maxime, Claude Opus 5.5 (1M context)
#### Miscellaneous Chores
- (**docs**) docs:deps and docs:build tasks, as the other operators (#24) - ([6498176](https://github.com/batleforc/weebo-authentik/commit/6498176e01b128d02ab6e00f2a1a3efb02401b55)) - Maxime, Claude Opus 5.5 (1M context)
- (**kit**) weebo-operator-kit v0.20.0 (#25) - ([c4b171c](https://github.com/batleforc/weebo-authentik/commit/c4b171c046c604752dc2e9553c508e7f14289983)) - Maxime, Claude Opus 5.5 (1M context)

- - -

## [v0.19.2](https://github.com/batleforc/weebo-authentik/compare/6d068212b8eba4db52ab977e58f7edd959866620..v0.19.2) - 2026-10-19
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.19.1, nested CRD fields in the reference (#23) - ([6d06821](https://github.com/batleforc/weebo-authentik/commit/6d068212b8eba4db52ab977e58f7edd959866620)) - Maxime, Claude Opus 5.5 (1M context)

- - -

## [v0.19.1](https://github.com/batleforc/weebo-authentik/compare/6a5338e7d149c9392c94b50953d0f04f6c60c7f4..v0.19.1) - 2026-10-19
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.18.2, 4 tokio workers by default (#22) - ([6a5338e](https://github.com/batleforc/weebo-authentik/commit/6a5338e7d149c9392c94b50953d0f04f6c60c7f4)) - Maxime, Claude Opus 5.5 (1M context)

- - -

## [v0.19.0](https://github.com/batleforc/weebo-authentik/compare/726f60f43f816942b4ba60f8d5a6cbcd4675d8a7..v0.19.0) - 2026-10-19
#### Features
- DriftCorrected Event and remote_writes_total, drift helpers from the kit (v0.18.1) (#20) - ([cf233ab](https://github.com/batleforc/weebo-authentik/commit/cf233abee64b7f4e7b6ca174bb3c32d515dd7af1)) - Maxime, Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context)
- update lock - ([726f60f](https://github.com/batleforc/weebo-authentik/commit/726f60f43f816942b4ba60f8d5a6cbcd4675d8a7)) - Max Batleforc

- - -

## [v0.18.3](https://github.com/batleforc/weebo-authentik/compare/da986b0e73268d8397f6f80cc480b9afadff507b..v0.18.3) - 2026-10-18
#### Bug Fixes
- (**gateway**) compare a scope mapping's text fields as Authentik stores them, trimmed (#19) - ([da986b0](https://github.com/batleforc/weebo-authentik/commit/da986b0e73268d8397f6f80cc480b9afadff507b)) - Maxime, Claude Opus 5.5 (1M context)

- - -

## [v0.18.2](https://github.com/batleforc/weebo-authentik/compare/89b4917a16290e32d534c6e7877148ad1871df5a..v0.18.2) - 2026-10-18
#### Bug Fixes
- (**gateway**) write only what drifted, never an unchanged object (#18) - ([89b4917](https://github.com/batleforc/weebo-authentik/commit/89b4917a16290e32d534c6e7877148ad1871df5a)) - Maxime, Claude Opus 5.5 (1M context)
#### Documentation
- (**perf**) resource budget measured on 2026-10-18 (#6) - ([faa7392](https://github.com/batleforc/weebo-authentik/commit/faa7392a60a1c73922daaa3c8942ac93117665b8)) - github-actions[bot], Claude Opus 5.5 (1M context), Max Batleforc, Claude Opus 5.5 (1M context), github-actions[bot]

- - -

## [v0.18.1](https://github.com/batleforc/weebo-authentik/compare/e194a54523ca3ddbbbacb2047994e5e12fc0c7c7..v0.18.1) - 2026-10-15
#### Bug Fixes
- (**application**) record a brand before switching the default; delete a recorded provider of any kind (#16) - ([e194a54](https://github.com/batleforc/weebo-authentik/commit/e194a54523ca3ddbbbacb2047994e5e12fc0c7c7)) - Maxime, Claude Opus 5.5
- (**deps**) kit v0.16.0 (#17) - ([faa17ed](https://github.com/batleforc/weebo-authentik/commit/faa17ed631ea17720bcf0bb5f350e2f56494a596)) - Maxime, Claude Opus 5.5

- - -

## [v0.18.0](https://github.com/batleforc/weebo-authentik/compare/faf9c4ca874edcb8c97d5c49f335ae6f9cb19bb2..v0.18.0) - 2026-10-14
#### Features
- (**vault**) pathGrants on the instance's Vault connection, kit v0.15.0 (#14) - ([faf9c4c](https://github.com/batleforc/weebo-authentik/commit/faf9c4ca874edcb8c97d5c49f335ae6f9cb19bb2)) - Maxime, Claude Opus 5.5
#### Bug Fixes
- (**deps**) rustls 0.23.46 (RUSTSEC-2026-0346..0349) (#15) - ([57a59df](https://github.com/batleforc/weebo-authentik/commit/57a59df0910aea722d101ab6d08c125e414fcf29)) - Maxime, Claude Opus 5.5

- - -

## [v0.17.2](https://github.com/batleforc/weebo-authentik/compare/622e6d561ab407bbc1619cffd4a830428ab17fb7..v0.17.2) - 2026-10-13
#### Bug Fixes
- (**application**) record remote objects early; delete a provider of any kind (#13) - ([622e6d5](https://github.com/batleforc/weebo-authentik/commit/622e6d561ab407bbc1619cffd4a830428ab17fb7)) - Maxime, Claude Opus 5.5

- - -

## [v0.17.1](https://github.com/batleforc/weebo-authentik/compare/c461c5138bf1a4e02f26d0b5c4d014d98ef40eff..v0.17.1) - 2026-10-13
#### Bug Fixes
- no orphan provider, e2e gates publishing, kit v0.14.1 (#12) - ([c461c51](https://github.com/batleforc/weebo-authentik/commit/c461c5138bf1a4e02f26d0b5c4d014d98ef40eff)) - Maxime, Claude Opus 5.5

- - -

## [v0.17.0](https://github.com/batleforc/weebo-authentik/compare/82ddcb6d42a164dbbfddcc5a9297ad5c893f26fe..v0.17.0) - 2026-10-13
#### Bug Fixes
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) review fixes of 2026-10-13, weebo-operator-kit v0.14.0 (#11) - ([f850911](https://github.com/batleforc/weebo-authentik/commit/f8509117255f89476ab7740a6ccfeda7b43ee621)) - Maxime, Claude Opus 5.5
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.13.0, amd64-only images (#10) - ([82ddcb6](https://github.com/batleforc/weebo-authentik/commit/82ddcb6d42a164dbbfddcc5a9297ad5c893f26fe)) - Maxime, Claude Opus 5.5

- - -

## [v0.16.2](https://github.com/batleforc/weebo-authentik/compare/3b270793939573c19dec89457593d5fc7adb26a8..v0.16.2) - 2026-10-13
#### Miscellaneous Chores
- (**kit**) weebo-operator-kit v0.12.1 (#9) - ([3b27079](https://github.com/batleforc/weebo-authentik/commit/3b270793939573c19dec89457593d5fc7adb26a8)) - Maxime, Claude Opus 5.5

- - -

## [v0.16.1](https://github.com/batleforc/weebo-authentik/compare/6beaffc860c5321fa4f78cd9ba94b025411fda01..v0.16.1) - 2026-10-13
#### Continuous Integration
- publish through the kit's workflow; Debian 13 and Hummingbird images (kit v0.11.0) (#8) - ([6beaffc](https://github.com/batleforc/weebo-authentik/commit/6beaffc860c5321fa4f78cd9ba94b025411fda01)) - Maxime, Claude Opus 5.5, Claude Opus 5.5, Claude Opus 5.5

- - -

## [v0.16.0](https://github.com/batleforc/weebo-authentik/compare/3f5ac213a47a547986f80d779ba92316fac297c8..v0.16.0) - 2026-10-12
#### Features
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) watch scope, AuthentikLocalInstance and instanceRef object (kit v0.10.1) (#7) - ([81aea41](https://github.com/batleforc/weebo-authentik/commit/81aea4154ef2770768525c0dd8f11f75716d0154)) - Maxime, Claude Opus 5.5, Claude Opus 5.5, Claude Opus 5.5, Claude Opus 5.5
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) build on weebo-operator-kit v0.9.0 (library chart, shared tasks, reusable CI) (#5) - ([75d4aa4](https://github.com/batleforc/weebo-authentik/commit/75d4aa4607db345d743ee87e1aeba273a73cdcff)) - Maxime
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) build on weebo-operator-kit v0.7.0 (#2) - ([97b3869](https://github.com/batleforc/weebo-authentik/commit/97b3869edab15eb3f05cf64c949baf778a7241e2)) - Maxime, Claude Opus 5.5
- rework docs (#1) - ([07b8875](https://github.com/batleforc/weebo-authentik/commit/07b887526e23f0d019c7934db62ffe3ecbbeb906)) - Maxime
- well follow the good practice for a default - ([2fc63fb](https://github.com/batleforc/weebo-authentik/commit/2fc63fb0c0b19450e4f243dfd818d30bd1308cbb)) - Max Batleforc
#### Documentation
- (**perf**) resource budget measured on 2026-10-11 (#4) - ([2679548](https://github.com/batleforc/weebo-authentik/commit/2679548e8f112dd3ab58e93ad8dbd1fb28491d69)) - github-actions[bot], github-actions[bot]
#### Continuous Integration
- (**perf**) measure the resource budget every week (kit v0.8.0) (#3) - ([1aa672f](https://github.com/batleforc/weebo-authentik/commit/1aa672f7cbc3f81832fffd078106b4ef0177f094)) - Maxime, Claude Opus 5.5
#### Miscellaneous Chores
- bump - ([3f5ac21](https://github.com/batleforc/weebo-authentik/commit/3f5ac213a47a547986f80d779ba92316fac297c8)) - Max Batleforc

- - -

## [v0.15.0](https://github.com/batleforc/proxyauthK8s/compare/5c6a1a8b38dd565612a1a9e038f1575562b3f1a0..v0.15.0) - 2026-09-20
#### Features
- Mise en place clientType public/confidential has confidential the default - ([5c6a1a8](https://github.com/batleforc/proxyauthK8s/commit/5c6a1a8b38dd565612a1a9e038f1575562b3f1a0)) - Max Batleforc

- - -

## [v0.14.0](https://github.com/batleforc/proxyauthK8s/compare/556f75aeec6bad0a70b124643edd9e8421655c0e..v0.14.0) - 2026-09-07
#### Features
- new policy - ([9333aae](https://github.com/batleforc/proxyauthK8s/commit/9333aaebb908429da7bf03e1c52316a0a707dc0c)) - Max Batleforc
#### Miscellaneous Chores
- bump - ([556f75a](https://github.com/batleforc/proxyauthK8s/commit/556f75aeec6bad0a70b124643edd9e8421655c0e)) - Max Batleforc

- - -

## [v0.13.0](https://github.com/batleforc/proxyauthK8s/compare/a1687a2abb0e8c3114768eae3c6ad1e448f8dc88..v0.13.0) - 2026-09-06
#### Features
- add device - ([7a5cf71](https://github.com/batleforc/proxyauthK8s/commit/7a5cf71a1c6d08d4573c45b019be878af9c5ccc7)) - Max Batleforc
#### Miscellaneous Chores
- bump version - ([a1687a2](https://github.com/batleforc/proxyauthK8s/commit/a1687a2abb0e8c3114768eae3c6ad1e448f8dc88)) - Max Batleforc

- - -

## [v0.12.0](https://github.com/batleforc/proxyauthK8s/compare/1ac85fb99d00eaceb2e50f1936e9f993db7d9657..v0.12.0) - 2026-09-06
#### Features
- Mise en place d'une nouvelle CRD dédié au scope mapping - ([25ad308](https://github.com/batleforc/proxyauthK8s/commit/25ad308c3bae9011bc0bcf10fa7bda0301155a21)) - Max Batleforc
#### Miscellaneous Chores
- bump - ([1ac85fb](https://github.com/batleforc/proxyauthK8s/commit/1ac85fb99d00eaceb2e50f1936e9f993db7d9657)) - Max Batleforc

- - -

## [v0.11.0](https://github.com/batleforc/proxyauthK8s/compare/b1ff4adef79fd9619408eba79436592ee17ee211..v0.11.0) - 2026-09-04
#### Features
- wip - ([ab9cc40](https://github.com/batleforc/proxyauthK8s/commit/ab9cc405cb1caa3642e95bd821d023eea9e7fb58)) - Max Batleforc
- extend the doc - ([b1ff4ad](https://github.com/batleforc/proxyauthK8s/commit/b1ff4adef79fd9619408eba79436592ee17ee211)) - Max Batleforc

- - -

## [v0.10.0](https://github.com/batleforc/proxyauthK8s/compare/94ff94d10cc3b096ade9e901733c558f10d677be..v0.10.0) - 2026-09-01
#### Features
- handle providing a CA - ([94ff94d](https://github.com/batleforc/proxyauthK8s/commit/94ff94d10cc3b096ade9e901733c558f10d677be)) - Max Batleforc

- - -

## [v0.9.0](https://github.com/batleforc/proxyauthK8s/compare/60c7f87a3853bb9ae37b5c3905a47abe97f72c68..v0.9.0) - 2026-09-01
#### Features
- wip - ([c00b49a](https://github.com/batleforc/proxyauthK8s/commit/c00b49ad209245cabb2578042f9c4c85987825a8)) - Max Batleforc
#### Bug Fixes
- let importer use Ca certificate - ([c6ea659](https://github.com/batleforc/proxyauthK8s/commit/c6ea659d8caf4c640f7b750d071aee263c011fe7)) - Max Batleforc
#### Miscellaneous Chores
- V0.8.0 - ([60c7f87](https://github.com/batleforc/proxyauthK8s/commit/60c7f87a3853bb9ae37b5c3905a47abe97f72c68)) - Max Batleforc

- - -

## [v0.8.0](https://github.com/batleforc/proxyauthK8s/compare/e03bf609c14e0e5ce1abb999b122bc8f7027952b..v0.8.0) - 2026-08-31
#### Features
- work on fixing thing that may block the migration - ([13c988d](https://github.com/batleforc/proxyauthK8s/commit/13c988d37bc4a2016dadc5b5bc9938be0145dd59)) - Max Batleforc
- update - ([e03bf60](https://github.com/batleforc/proxyauthK8s/commit/e03bf609c14e0e5ce1abb999b122bc8f7027952b)) - Max Batleforc

- - -

## [v0.7.0](https://github.com/batleforc/proxyauthK8s/compare/510f2f8d023e2a57b8cfb90dbb1385afd4332139..v0.7.0) - 2026-08-31
#### Features
- enhance the test - ([ff0248e](https://github.com/batleforc/proxyauthK8s/commit/ff0248e92aff50b3f98297be4dfc3f5973ce8df0)) - Max Batleforc
- add handling of certificate and flow - ([510f2f8](https://github.com/batleforc/proxyauthK8s/commit/510f2f8d023e2a57b8cfb90dbb1385afd4332139)) - Max Batleforc

- - -

## [v0.6.0](https://github.com/batleforc/proxyauthK8s/compare/4501202d5a8526a14a8e9671b22ec05dfa412a3e..v0.6.0) - 2026-08-31
#### Features
- doc and cert - ([4501202](https://github.com/batleforc/proxyauthK8s/commit/4501202d5a8526a14a8e9671b22ec05dfa412a3e)) - Max Batleforc

- - -

## [v0.5.0](https://github.com/batleforc/proxyauthK8s/compare/04447aecf316a3348166c789b47b23973045bcf6..v0.5.0) - 2026-08-24
#### Features
- WIP - ([e158810](https://github.com/batleforc/proxyauthK8s/commit/e1588101ef52cf999d70020d0b94bc5cb2089e40)) - Max Batleforc
- work on fixing the E2E test - ([ccbd532](https://github.com/batleforc/proxyauthK8s/commit/ccbd532f5cf6a0c80abdc5b522514e204d591198)) - Max Batleforc
- --skip-crd - ([03757f0](https://github.com/batleforc/proxyauthK8s/commit/03757f0511a956852e620c0cb11458e6600c8b4e)) - Max Batleforc
- make the ci/cd work (i hope) - ([04447ae](https://github.com/batleforc/proxyauthK8s/commit/04447aecf316a3348166c789b47b23973045bcf6)) - Max Batleforc

- - -

## [v0.4.0](https://github.com/batleforc/proxyauthK8s/compare/e14c7f6c1d0f587545c941cb191b13a0bdf85e64..v0.4.0) - 2026-08-16
#### Features
- add doc - ([b568466](https://github.com/batleforc/proxyauthK8s/commit/b568466b38fad0dac8e76e210af11a35e335f096)) - Max Batleforc
- mise en place cache - ([b4edfab](https://github.com/batleforc/proxyauthK8s/commit/b4edfabc4e9d08955f27d10a5fd1ea67269988fb)) - Max Batleforc
- wip - ([68fa37c](https://github.com/batleforc/proxyauthK8s/commit/68fa37ce0d5cee03c812408dd10fdab0cc397ac6)) - Max Batleforc
- wip - ([017ef28](https://github.com/batleforc/proxyauthK8s/commit/017ef2893f020cffe59de20960de1a46038b57c1)) - Max Batleforc
- wip ci - ([6ccca23](https://github.com/batleforc/proxyauthK8s/commit/6ccca23dd1aa5c840c846a94144a398c0ec545a4)) - Max Batleforc
- wip - ([e14c7f6](https://github.com/batleforc/proxyauthK8s/commit/e14c7f6c1d0f587545c941cb191b13a0bdf85e64)) - Max Batleforc

- - -

## [v0.3.0](https://github.com/batleforc/proxyauthK8s/compare/7d2a422946348101f12b7fe8067b68b4f5bb83a0..v0.3.0) - 2026-07-18
#### Features
- wip - ([f263f0a](https://github.com/batleforc/proxyauthK8s/commit/f263f0a7e08b8704c965797973d49fec0e4bd7c8)) - Max Batleforc
- wip - ([7d2a422](https://github.com/batleforc/proxyauthK8s/commit/7d2a422946348101f12b7fe8067b68b4f5bb83a0)) - Max Batleforc

- - -

## [v0.2.0](https://github.com/batleforc/proxyauthK8s/compare/4877b2c893bf856dcfd4e51087b5b89d2e4f0d7d..v0.2.0) - 2026-07-17
#### Features
- wip - ([4877b2c](https://github.com/batleforc/proxyauthK8s/commit/4877b2c893bf856dcfd4e51087b5b89d2e4f0d7d)) - Max Batleforc

- - -

## [v0.1.0](https://github.com/batleforc/proxyauthK8s/compare/167afa3a2ad9bc6f3975db46bbc771aff8ac242f..v0.1.0) - 2026-07-17
#### Features
- ci/cd - ([f72569a](https://github.com/batleforc/proxyauthK8s/commit/f72569acf27f79a3a00032aadfdb29a91d734da4)) - Max Batleforc
- force the profile - ([83f68f3](https://github.com/batleforc/proxyauthK8s/commit/83f68f30996ed4fd855b8c1939977be4b595808c)) - Max Batleforc
- add rust ftmt - ([cb1393a](https://github.com/batleforc/proxyauthK8s/commit/cb1393a5f61e88cbb0e5044cfbbbe5dc672006a2)) - Max Batleforc
- wip - ([f9c1fa2](https://github.com/batleforc/proxyauthK8s/commit/f9c1fa2da2daea9e1d5b724eaf85a0ef5eb9178f)) - Max Batleforc
- enhance the doc - ([da988e1](https://github.com/batleforc/proxyauthK8s/commit/da988e14820adc2ff92e5b537d49de772f10bfe1)) - Max Batleforc
- Mise en place doc - ([3e4e364](https://github.com/batleforc/proxyauthK8s/commit/3e4e36403e5e1a883c4952ed270411aed63a040b)) - Max Batleforc
- wip - ([2089e0c](https://github.com/batleforc/proxyauthK8s/commit/2089e0cec84645c05b5d79037b5ca98203a033fd)) - Max Batleforc
- Scaffold hexagonal-architecture skeleton for the Authentik operator - ([12dee71](https://github.com/batleforc/proxyauthK8s/commit/12dee712dd2860fc388b3fba2bc7b45f9db39a59)) - Max Batleforc, Claude Sonnet 5
- Add planning docs for the Authentik Kubernetes operator - ([493cafe](https://github.com/batleforc/proxyauthK8s/commit/493cafed9a586a8d8b5d4cc05f0f0ca296c11740)) - Max Batleforc, Claude Sonnet 5
- Rename project scaffold to weebo-authentik and add Rust .gitignore - ([d2492ee](https://github.com/batleforc/proxyauthK8s/commit/d2492ee6ff8a77f4d47b4ea498f9e2d9b5ae683f)) - Max Batleforc, Claude Sonnet 5
- Initial commit - ([167afa3](https://github.com/batleforc/proxyauthK8s/commit/167afa3a2ad9bc6f3975db46bbc771aff8ac242f)) - Maxime
#### Bug Fixes
- cicd - ([5b7c5d2](https://github.com/batleforc/proxyauthK8s/commit/5b7c5d2565378dad1c6f500d61d16efb2ddcd5de)) - Max Batleforc
- test suit - ([8948be6](https://github.com/batleforc/proxyauthK8s/commit/8948be6780261f87d3028062487d4cb538e3a1a1)) - Max Batleforc

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).