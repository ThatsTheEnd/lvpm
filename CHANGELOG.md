# Changelog

## [0.27.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.26.0...v0.27.0) (2026-11-12)


### ⚠ BREAKING CHANGES

* **venv:** Existing projects using .project must set venv.dir to .project (or set LVPM_CONFIG_VENV_DIR=.project) to keep using their existing venv.
* **config:** local directory entries in project `[sources]` are rejected; configure them globally instead.
* **cli:** `--repo` is no longer accepted; use `[sources]` in `lvpm.toml` instead.

### Features

* [sources] min-age-days holds back too-new package versions ([#101](https://github.com/ThatsTheEnd/lvpm/issues/101)) ([e08e2ab](https://github.com/ThatsTheEnd/lvpm/commit/e08e2ab6e0199c2f6267f121088d4d995dd56ba1))
* add commands to start LabVIEW and run PostInstall hooks ([eb7e688](https://github.com/ThatsTheEnd/lvpm/commit/eb7e6885e79e0a5b7c6bbbf3eea22792db77fc2c))
* Added typed VI Server client and vi-run command ([988ea99](https://github.com/ThatsTheEnd/lvpm/commit/988ea997e8c34133b5bad415ffcfac53f8d369ce))
* **cache:** acquire install archives through the package cache ([ec0e0e0](https://github.com/ThatsTheEnd/lvpm/commit/ec0e0e0518bdd80a3d1281ff85346b8d6d45fd86))
* **cache:** import local package archives with cache add ([2076227](https://github.com/ThatsTheEnd/lvpm/commit/2076227ab80de67c06dbe27acc9a4b7ae449167c))
* **cache:** index every source in SQLite and revalidate feeds ([fd3d1c5](https://github.com/ThatsTheEnd/lvpm/commit/fd3d1c547e8af3638b17c90e8a2da008418b2618))
* **cache:** record the SQLite schema version ([49e4d3a](https://github.com/ThatsTheEnd/lvpm/commit/49e4d3acb52c58219a09476578ec0b6db16488b6))
* **cache:** share package archives across projects and installations ([812fcae](https://github.com/ThatsTheEnd/lvpm/commit/812fcae4bec5b9a51626677c120f7e6ce5ea37fc))
* **cli:** remove the --repo flag ([4890512](https://github.com/ThatsTheEnd/lvpm/commit/4890512a5d5da038c8464838fff83a1bd6ea887a))
* **config:** move local package sources out of manifests ([1b12749](https://github.com/ThatsTheEnd/lvpm/commit/1b127499226c80904c81b50fd0328174912bb79a))
* **config:** resolve cache location from LVPM_CONFIG_CACHE ([4a0bffc](https://github.com/ThatsTheEnd/lvpm/commit/4a0bffc92f3ba3705ba97b72f63e7eabee0e362d))
* **config:** resolve cache location from LVPM_CONFIG_CACHE ([8b124c4](https://github.com/ThatsTheEnd/lvpm/commit/8b124c4497cb44dae3af6d2aa58ae607a59254ac))
* **core:** add a PubGrub solver for project locking ([add2df1](https://github.com/ThatsTheEnd/lvpm/commit/add2df10bb7dd89d0380f9bb4f876b5525cb6f1b))
* **core:** add and remove dependencies with lock and sync ([bcc36cc](https://github.com/ThatsTheEnd/lvpm/commit/bcc36cc773fcade994d529f360729b7b1beefdeb))
* **core:** cancel operations before they write files ([4a5a914](https://github.com/ThatsTheEnd/lvpm/commit/4a5a9148943ba2fa86a89e3dffff75ff3cec1093))
* **core:** define the lvpm.lock format ([0cd2541](https://github.com/ThatsTheEnd/lvpm/commit/0cd25410a436c5013e308a58d85a91c9f02e4e8c))
* **core:** detect stale locks with a reason ([a20f876](https://github.com/ThatsTheEnd/lvpm/commit/a20f87688efdebb35fd12ec1336f84d9f5799f53))
* **core:** edit lvpm.toml entries without touching the rest of the file ([2895668](https://github.com/ThatsTheEnd/lvpm/commit/2895668ca5728509ca89d6a6168b5a4e3f9e8c0e))
* **core:** explain solver conflicts and report unresolved requirements ([e24a3dc](https://github.com/ThatsTheEnd/lvpm/commit/e24a3dc432f97dfff0173856596c3dbf803296e0))
* **core:** fetch locked packages by SHA-256 from any source ([2a60743](https://github.com/ThatsTheEnd/lvpm/commit/2a6074371dbf77aabd3d8e3abd539bf30ec79dca))
* **core:** plan an exact sync from lvpm.lock ([861b1ab](https://github.com/ThatsTheEnd/lvpm/commit/861b1ab93272f41cc162490a03ffa0fd3a243857))
* **core:** read [dev-dependencies] from lvpm.toml ([e46c505](https://github.com/ThatsTheEnd/lvpm/commit/e46c505328e145366c77c20102e4163dc23c6ace))
* embed Relink Package.vi in the binary ([c941370](https://github.com/ThatsTheEnd/lvpm/commit/c9413708a493b2ee5c4ecb38ced788e6f858c367))
* enhance VI Server protocol to support one-dimensional arrays ([18448fa](https://github.com/ThatsTheEnd/lvpm/commit/18448fa78f4955ee6103845127268956eb494ad5))
* headless lvpm sync requires a current lock ([41f9b75](https://github.com/ThatsTheEnd/lvpm/commit/41f9b75ad0a274be2a2e13fe526c7467fae605b8))
* hooks run in a venv, and the relink pass becomes opt-in ([07236ca](https://github.com/ThatsTheEnd/lvpm/commit/07236ca947d8edd2fa5a7984d4144cb90834594a))
* **init:** lvpm init and init --from .vipc / vipm.toml ([#94](https://github.com/ThatsTheEnd/lvpm/issues/94)) ([a05b866](https://github.com/ThatsTheEnd/lvpm/commit/a05b866c3d1ba2b7dce5e0fa05ed77dae09fbb38))
* **installer:** support local release installation ([39a5aaf](https://github.com/ThatsTheEnd/lvpm/commit/39a5aafa50dd1994121c3ddff3de9e0ea60680d9))
* **installer:** support local release installation ([99e1c68](https://github.com/ThatsTheEnd/lvpm/commit/99e1c68290f65167f253bdb2d98266fb25998ab6))
* **linkfix:** make installed VIs source-only ([0121fc0](https://github.com/ThatsTheEnd/lvpm/commit/0121fc0cfc7eb859fee4b2d07240c8d190673628))
* **linkfix:** select methods with --fix, write --report, harden source-only ([e18e9db](https://github.com/ThatsTheEnd/lvpm/commit/e18e9db069a7d2010a14911bb4fae03e724fa5e5))
* **linux:** drive LabVIEW for Linux; end-to-end tests on Linux and Windows ([8e8bd7e](https://github.com/ThatsTheEnd/lvpm/commit/8e8bd7e390b7d4a77558af12cfa52f79079df409))
* lvpm add / lvpm remove; lvpm install &lt;pkg&gt; adds in a project ([ef5c5fe](https://github.com/ThatsTheEnd/lvpm/commit/ef5c5fe65b6412b52d4da0cee1b500b9ce6cbbea))
* lvpm linkfix — the relink pass in Rust, no LabVIEW ([b89bf60](https://github.com/ThatsTheEnd/lvpm/commit/b89bf60587c8d00635a3b278d5430790c2113e8d))
* lvpm lock --check ([8c11dba](https://github.com/ThatsTheEnd/lvpm/commit/8c11dbac29bfab1ef0de7712e38fedd5059c706f))
* lvpm lock --upgrade and --upgrade-package ([7edb5cb](https://github.com/ThatsTheEnd/lvpm/commit/7edb5cb4b014fe9b996938cf283f1bfec0330705))
* lvpm lock writes lvpm.lock from the solver ([76e20c7](https://github.com/ThatsTheEnd/lvpm/commit/76e20c72143dd8d9fad23c060c64dbbfc47d65db))
* lvpm publish --to &lt;folder&gt; (S2) ([#55](https://github.com/ThatsTheEnd/lvpm/issues/55)) ([8a5586b](https://github.com/ThatsTheEnd/lvpm/commit/8a5586b8612ccf95c1bcfc9220acd0010784c02a))
* lvpm sync; bare lvpm install syncs the project to lvpm.lock ([c160070](https://github.com/ThatsTheEnd/lvpm/commit/c1600708770dffd717492ab87a1ff8a067942848))
* lvpm-tui plan and Slice 1, lvpm search --all (0.6.0) ([#45](https://github.com/ThatsTheEnd/lvpm/issues/45)) ([0e0fcb2](https://github.com/ThatsTheEnd/lvpm/commit/0e0fcb2f9c6409550cac5e7c9441bab3baa754f3))
* lvpm.lock (uv model) and the lvpm-core extraction ([800b95e](https://github.com/ThatsTheEnd/lvpm/commit/800b95e3888b2a3842cdd38b3ec9b9b365a921cc))
* lvpm.toml — own manifest with [sources] and version constraints ([6f925cb](https://github.com/ThatsTheEnd/lvpm/commit/6f925cb8e8fc09d37bf93f0a4ab6937ac381685c))
* manifest installs, relink --all, and install-hook execution ([f3f9955](https://github.com/ThatsTheEnd/lvpm/commit/f3f99554c48d52187716571063a66da7d69ceb7c))
* **manifest:** add dependency groups with --with, --without and --only ([acf482e](https://github.com/ThatsTheEnd/lvpm/commit/acf482ee45e54b63577e3a8ae0acc199e7e58f46))
* **manifest:** nest dependency groups under dependencies ([08f8682](https://github.com/ThatsTheEnd/lvpm/commit/08f8682c35d9644abd9f4dd83cd010ed2cc5727d))
* merge the public indexes, TUI install adds to lvpm.toml, lvpm cache refresh ([#124](https://github.com/ThatsTheEnd/lvpm/issues/124)) ([428589e](https://github.com/ThatsTheEnd/lvpm/commit/428589e094197272ab1ed79b58121708b213e797))
* per-project dependencies — venvs mounted through LVAddons ([2935e0a](https://github.com/ThatsTheEnd/lvpm/commit/2935e0a90801fc62f64a2548b6fc95155f1b6bfa))
* port Flydroid/lvpm main (8e8bd7e..bdaf82d): linkfix, dependency uninstall, venv layout ([63df621](https://github.com/ThatsTheEnd/lvpm/commit/63df621a119f766147aaee017bda06f9966e044d))
* **project:** parse project dependency manifests ([759ccfe](https://github.com/ThatsTheEnd/lvpm/commit/759ccfeec7d4ea8c445e54803803c2d595b779aa))
* refresh palettes and menus after install, uninstall and hooks ([edcb240](https://github.com/ThatsTheEnd/lvpm/commit/edcb240055335bb3876a0e47b440b371492c478a))
* relink packages after install, support local .vip repos ([e18f7f0](https://github.com/ThatsTheEnd/lvpm/commit/e18f7f0bfb94449eb0c2c64d38af414f5de02ac1))
* run uninstall hooks ([09a1b70](https://github.com/ThatsTheEnd/lvpm/commit/09a1b70b0eec528106be76dfd7fbc4e5f4261fbc))
* **s10d:** --yes, --labview-bitness, no lvpm update ([#106](https://github.com/ThatsTheEnd/lvpm/issues/106)) ([3b51c59](https://github.com/ThatsTheEnd/lvpm/commit/3b51c59d472dfa4f6e3002fb79861e0cd1772958))
* **s11:** dependency groups ([#104](https://github.com/ThatsTheEnd/lvpm/issues/104)) ([29eef60](https://github.com/ThatsTheEnd/lvpm/commit/29eef607a1493e9b155e6f9bb97c32e105dce0ff))
* **s12:** install hooks off by default ([#118](https://github.com/ThatsTheEnd/lvpm/issues/118)) ([efffcdc](https://github.com/ThatsTheEnd/lvpm/commit/efffcdc647b30c2fd4f09a08238f1d39ab937055))
* **s4:** GitHub Releases as a feed ([#98](https://github.com/ThatsTheEnd/lvpm/issues/98)) ([a3c376c](https://github.com/ThatsTheEnd/lvpm/commit/a3c376c33333cf8c75280aa20165cce4b1895051))
* **s5b:** lvpm build runs the check ([#114](https://github.com/ThatsTheEnd/lvpm/issues/114)) ([7c53245](https://github.com/ThatsTheEnd/lvpm/commit/7c5324565bd6f0bcd47daef7841c8c387f0b7c48))
* **s5:** lvpm check ([#109](https://github.com/ThatsTheEnd/lvpm/issues/109)) ([690d939](https://github.com/ThatsTheEnd/lvpm/commit/690d9393dd4c440416a762f441bb39e60f7832d4))
* **s6:** attestations and consumer policy ([#112](https://github.com/ThatsTheEnd/lvpm/issues/112)) ([5ee9d47](https://github.com/ThatsTheEnd/lvpm/commit/5ee9d47dfe216d226069902a7b2649ba93a71d36))
* **s7a:** lvpm build writes a .lvpkg (ADR 6) ([#84](https://github.com/ThatsTheEnd/lvpm/issues/84)) ([91fb780](https://github.com/ThatsTheEnd/lvpm/commit/91fb780cac83857f1511048026dcf0f03c389f15))
* **s7b:** OCI feeds ([#121](https://github.com/ThatsTheEnd/lvpm/issues/121)) ([c98e2a3](https://github.com/ThatsTheEnd/lvpm/commit/c98e2a3032bd15ce61beab8704ab81ab1a7b80c8))
* **s7c:** pin a dependency to one source ([#116](https://github.com/ThatsTheEnd/lvpm/issues/116)) ([aab9a58](https://github.com/ThatsTheEnd/lvpm/commit/aab9a58efa15ef42063b7130358e9c0d35b6c260))
* **s8:** federated community index ([#122](https://github.com/ThatsTheEnd/lvpm/issues/122)) ([c941b4e](https://github.com/ThatsTheEnd/lvpm/commit/c941b4e2617d8bea2b96e6c74dd718cf1bdebb3f))
* **s9:** application builds and SBOM ([#119](https://github.com/ThatsTheEnd/lvpm/issues/119)) ([2db838e](https://github.com/ThatsTheEnd/lvpm/commit/2db838e4582c25e85542841795c58c93220d0e1b))
* **sources:** the first source wins for every command (S13) ([#83](https://github.com/ThatsTheEnd/lvpm/issues/83)) ([f3cbed8](https://github.com/ThatsTheEnd/lvpm/commit/f3cbed801d8257b5db3e3075e84b244f1c3bc5f7))
* **tui:** lvpm-tui slices 4-7 on main (0.11.0) ([#56](https://github.com/ThatsTheEnd/lvpm/issues/56)) ([1e61b4b](https://github.com/ThatsTheEnd/lvpm/commit/1e61b4b0716dde5cdf5f69ac00b48a27b4305af2))
* **tui:** ship the TUI as `lvpm tui`, fix its rough edges, and have init write a .gitignore ([#96](https://github.com/ThatsTheEnd/lvpm/issues/96)) ([994026b](https://github.com/ThatsTheEnd/lvpm/commit/994026be672b2adaa9010f9b5c3a63017cb4b0a4))
* uninstall --all removes every package in the target ([2679f3a](https://github.com/ThatsTheEnd/lvpm/commit/2679f3a9b7ccc174f4958432d3be283f350ef984))
* update VI Server protocol documentation and improve control value retrieval methods ([cfa357c](https://github.com/ThatsTheEnd/lvpm/commit/cfa357c344686cbcd4a0b3b97616867e209c4588))
* upgrade a locked package to a chosen version ([588f7e8](https://github.com/ThatsTheEnd/lvpm/commit/588f7e8799d52ce00b8cd1c5b0d3c0cc04fe16ac))
* upgrade installed packages, refuse downgrades unless asked ([598928e](https://github.com/ThatsTheEnd/lvpm/commit/598928e7cf413dc42442d74c2a63ed92069f1097))
* **venv:** give each venv its own cache, LabVIEW Data and temp ([bf4edf0](https://github.com/ThatsTheEnd/lvpm/commit/bf4edf0d8bd300030de8436a7b785b71b8e469ee))
* **venv:** install resaves packages in the venv's LabVIEW before they land ([#81](https://github.com/ThatsTheEnd/lvpm/issues/81)) ([0e39def](https://github.com/ThatsTheEnd/lvpm/commit/0e39defadaf26937dc76b70bb6bca989e0be40ce))
* **venv:** keep the venv's own ini, never start a second LabVIEW on it ([3c39417](https://github.com/ThatsTheEnd/lvpm/commit/3c39417a2b4be40710a20f81cd1a96a007ad4036))
* **venv:** make project venv directory configurable ([508a6a9](https://github.com/ThatsTheEnd/lvpm/commit/508a6a929ead6b8b3b0fbce1f9568040e029e40f))
* vi-probe/vi-save/vi-run talk to the venv's LabVIEW when a venv is in effect ([1e9a92e](https://github.com/ThatsTheEnd/lvpm/commit/1e9a92eed47090f3b2a51acec5aa99a5dbce3f01))
* **vipm:** read vipm.toml dev-dependencies as group dev ([1f6c658](https://github.com/ThatsTheEnd/lvpm/commit/1f6c6587179364714605e1b54ed24624a426a1df))
* **vipm:** read vipm.toml through --manifest ([ecbfb6c](https://github.com/ThatsTheEnd/lvpm/commit/ecbfb6cb75a964387af2f2332459ff10dda68f78))
* **viserver:** decode paths and variant attributes, type server errors ([27a60fb](https://github.com/ThatsTheEnd/lvpm/commit/27a60fb71abe155d4eac42f09fa675d3b619ca80))


### Bug Fixes

* a number inside a package name is not the version ([1f816e8](https://github.com/ThatsTheEnd/lvpm/commit/1f816e8ae44922ed3d4e01ffd9dfd5a5304aa300))
* a number inside a package name is not the version ([fa43874](https://github.com/ThatsTheEnd/lvpm/commit/fa43874a991976f4e971eb8580e1f521a9c91e30))
* **add:** stop accepting a cancel before the manifest and lock are written ([1ff29ca](https://github.com/ThatsTheEnd/lvpm/commit/1ff29ca8e4d9dd28e0ab5ef0e4265e89beffab28))
* **cache:** migrate an old cache under the write lock ([5ecca68](https://github.com/ThatsTheEnd/lvpm/commit/5ecca680da67fb8212c16cdb92d503ed58e6c46e))
* **cancel:** refuse a cancel once lock, upgrade or add/remove writes a file ([c7a1ccf](https://github.com/ThatsTheEnd/lvpm/commit/c7a1ccf0be4228c4e06fb2e3aa050f6ae973e99f))
* **core:** compare prerelease tails instead of discarding them ([0227c0a](https://github.com/ThatsTheEnd/lvpm/commit/0227c0a1acf366df0defcad5a66dfddce58b3f55))
* **core:** keep comment lines when removing a manifest entry ([6347533](https://github.com/ThatsTheEnd/lvpm/commit/63475330425e6f41199bc5262eee1b44fe1937fe))
* **core:** lock a package listed in both dependency tables once ([6ffbc2d](https://github.com/ThatsTheEnd/lvpm/commit/6ffbc2dc73fc81451b8afec442225b61a7e51d7f))
* **core:** memoise conflict facts and dedup unresolved case-insensitively ([9fd3d0c](https://github.com/ThatsTheEnd/lvpm/commit/9fd3d0cf788b09fe6f32f8d20ee0ae0390b969b1))
* **core:** sort a prerelease below its release even with a revision ([a82451e](https://github.com/ThatsTheEnd/lvpm/commit/a82451eafb33d5ccb42a5e2c1468eb0d0f8010f0))
* **core:** take config set and cache add paths from the injected cwd ([b44e711](https://github.com/ThatsTheEnd/lvpm/commit/b44e71198b089198a1d451c3ca08461eef040fa1))
* **core:** treat a &gt; requirement as strict, not as &gt;= ([efcc104](https://github.com/ThatsTheEnd/lvpm/commit/efcc104a9c497cd2968d04ee5c9008c3f49ff9c0))
* **e2e:** the public suite's baseline is the tree after a first start ([f4d4b10](https://github.com/ThatsTheEnd/lvpm/commit/f4d4b109f909c782c0a504edc6b3046c9b48b530))
* **e2e:** the relink VI is in %TEMP% on Windows ([e59606e](https://github.com/ThatsTheEnd/lvpm/commit/e59606e9bd95b9072e086587fb11cf12c70c0266))
* **errors:** give a cancelled sync, add and remove one neutral shape ([33de012](https://github.com/ThatsTheEnd/lvpm/commit/33de012a83ef839d5b76f57a015fa5f7aa30554b))
* **help:** name the manifest key venv create reads as labview ([d32c443](https://github.com/ThatsTheEnd/lvpm/commit/d32c443e06e2544f1a00da5da876914f740c82b1))
* **hooks:** refuse a hook VI outside the hooks directory and keep the PostUninstall copy in the temp directory ([3cec134](https://github.com/ThatsTheEnd/lvpm/commit/3cec1345728519a6346436d3238c731d3e90688d))
* **install:** a global upgrade keeps the old version when the new archive is refused ([#85](https://github.com/ThatsTheEnd/lvpm/issues/85)) ([b333667](https://github.com/ThatsTheEnd/lvpm/commit/b333667a4e00e6ec9ceabc6d31355598cf8b1d49))
* **install:** a relative prefix that starts with .. is a place to write, not outside it ([43fb24f](https://github.com/ThatsTheEnd/lvpm/commit/43fb24f54f1aeda5e07a46c3e8f968da73e8ed47))
* **install:** pick the newest release again ([a921414](https://github.com/ThatsTheEnd/lvpm/commit/a921414fe166939f30459046930755f3df15982d))
* **install:** pick the newest release again ([c73ff54](https://github.com/ThatsTheEnd/lvpm/commit/c73ff548d2da36d6c91898afe0bcdab194011c28))
* **install:** print "lvpm &lt;version&gt; installed" as one line ([291e1b4](https://github.com/ThatsTheEnd/lvpm/commit/291e1b495c539b30d849bf38fdc092590facf232))
* **install:** read name@constraint the same way everywhere ([07110aa](https://github.com/ThatsTheEnd/lvpm/commit/07110aa210a0f223d7636025ddb885b7f31f2ab3))
* **install:** refuse to write where a record may not name, the boot volume root included ([ad92cb0](https://github.com/ThatsTheEnd/lvpm/commit/ad92cb04309d070b9e32fc64d7a9ec21821373ba))
* LabVIEW 2025 relinking, and the installer version print ([cb7822a](https://github.com/ThatsTheEnd/lvpm/commit/cb7822abe4a24b14119b8204db46d2f5d3afb147))
* **linkfix:** find build-machine relative links by qualified name ([aa7c9e4](https://github.com/ThatsTheEnd/lvpm/commit/aa7c9e48b19f161d2fe9e3649bf4fe4169a46989))
* **linkfix:** keep the class in qualified names of class private data ([6045021](https://github.com/ThatsTheEnd/lvpm/commit/6045021b754e7b69f4ffe45f95886a2cccc18ff8))
* **linkfix:** search block-diagram password salts up to 256 ([e64eb8c](https://github.com/ThatsTheEnd/lvpm/commit/e64eb8c424076037c88898f52c0d633804c9e3cc))
* **linkfix:** separate LabVIEW 2015 VIs; report unknown tag layouts ([6ebb21a](https://github.com/ThatsTheEnd/lvpm/commit/6ebb21adccec389b4d9922e7767d5926312b6b27))
* **linkfix:** use path tokens only where LabVIEW searches add-ons ([8a9b2a4](https://github.com/ThatsTheEnd/lvpm/commit/8a9b2a474ffb4f5e3bdad9a015ca2d8a6f0e30ec))
* **lock:** keep a locked version's SHA-256 when the lock is made again ([bec755d](https://github.com/ThatsTheEnd/lvpm/commit/bec755d766e8c205de2bacc64db6cd7b37fec5bd))
* **lock:** stage each lvpm.lock write to a file of its own ([5bdd14b](https://github.com/ThatsTheEnd/lvpm/commit/5bdd14bf41c79a5d2eb1b22f7ea996a1b84f568a))
* name upgrade pins in conflict reports ([6af600b](https://github.com/ThatsTheEnd/lvpm/commit/6af600bf12ca08c6fc4345b8e3e0c32d9040e981))
* record If Newer files the install left alone in the manifest ([9a4fc48](https://github.com/ThatsTheEnd/lvpm/commit/9a4fc48e69dd1dfba78eef2e534cc9f2c1d3cf9a))
* **relink:** bundle the relink VI with the application ([2a56a4b](https://github.com/ThatsTheEnd/lvpm/commit/2a56a4b969197a7b1fc8fda3707d576fe7be4361))
* **relink:** native separators, error-1000 retries, cross-package folder dedup ([50e6fe3](https://github.com/ThatsTheEnd/lvpm/commit/50e6fe398a00d48a1d5b1402ca0054b6d900f914))
* **relink:** skip shared Providers tree and update Linux E2E checks ([adf108d](https://github.com/ThatsTheEnd/lvpm/commit/adf108d5d0fa288ed2f83bff03e25a610e729956))
* **solve:** settle an upgrade's targets before the packages held at their locked versions ([faa0a77](https://github.com/ThatsTheEnd/lvpm/commit/faa0a77c44030f3c05100c09d2ad7ee8e351f420))
* **sync:** a cancel during the last source's load is a cancel, and an unfetchable kept archive says why ([755a9a2](https://github.com/ThatsTheEnd/lvpm/commit/755a9a2fcb4c6f024b974fd5743adb849f3ed6fc))
* **sync:** keep an installed package the lock records as unresolved ([39eee6e](https://github.com/ThatsTheEnd/lvpm/commit/39eee6e2279a69a181ab8aaf542dd8dd2979fbbd))
* **sync:** replace an install that records no archive SHA-256 ([84e42bd](https://github.com/ThatsTheEnd/lvpm/commit/84e42bd369b60a98a131a62d9c1d692d02390c5b))
* **sync:** warn once about a requirement no source lists ([4ca5edb](https://github.com/ThatsTheEnd/lvpm/commit/4ca5edb8832e636fc12f4c1cba653496df36a28e))
* **tests:** update expected output format for LabVIEW ini configuration ([7200ff2](https://github.com/ThatsTheEnd/lvpm/commit/7200ff2c40e298d16f5ebc7fadf9886deaef1f11))
* **uninstall:** refuse a cancel once uninstall deletes its first file ([ef51bc2](https://github.com/ThatsTheEnd/lvpm/commit/ef51bc29146ed0cfdc6e351b9e2d2b79eadc24dc))
* **uninstall:** refuse an install record that names a path outside the destination ([475bde1](https://github.com/ThatsTheEnd/lvpm/commit/475bde167735e77026e148b522a39cd64d81aec6))
* update repository URL in Cargo.toml, add Apache and MIT licenses, and enhance README.md ([7e2c482](https://github.com/ThatsTheEnd/lvpm/commit/7e2c482ad9dbe67630036db306a41f34f8e0899c))
* **venv:** leave recent files out of the venv ini, write it before the binding ([06a4597](https://github.com/ThatsTheEnd/lvpm/commit/06a4597781e9eebe64325c92d3ba18ce230810b2))
* **venv:** read the manifest's labview the way locking does ([e7004c1](https://github.com/ThatsTheEnd/lvpm/commit/e7004c1e5234b91b312518b30c3c5f05e5177751))
* **viserver:** flatten variants at 2020, not 2026 ([b9e7981](https://github.com/ThatsTheEnd/lvpm/commit/b9e7981673da6d673e916c7af6c989047c311129))


### Performance Improvements

* **cache:** reuse local package metadata when file stats match ([799d83d](https://github.com/ThatsTheEnd/lvpm/commit/799d83d6a1d2faf6214b01ff4fc0abb8f933fd7b))
* **linkfix:** fix files in parallel ([2667d0d](https://github.com/ThatsTheEnd/lvpm/commit/2667d0d07a938f86a944c872746da0457e4ba1cd))


### Reverts

* **linkfix:** drop the old-class rule ([972f007](https://github.com/ThatsTheEnd/lvpm/commit/972f007cc05ce5dba507c14b4d015e22bacbc84a))

## [0.26.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.25.0...v0.26.0) (2026-11-12)


### Features

* merge the public indexes, TUI install adds to lvpm.toml, lvpm cache refresh ([#124](https://github.com/ThatsTheEnd/lvpm/issues/124)) ([428589e](https://github.com/ThatsTheEnd/lvpm/commit/428589e094197272ab1ed79b58121708b213e797))

## [0.25.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.24.0...v0.25.0) (2026-11-08)


### Features

* port Flydroid/lvpm main (8e8bd7e..bdaf82d): linkfix, dependency uninstall, venv layout ([63df621](https://github.com/ThatsTheEnd/lvpm/commit/63df621a119f766147aaee017bda06f9966e044d))
* **s7b:** OCI feeds ([#121](https://github.com/ThatsTheEnd/lvpm/issues/121)) ([c98e2a3](https://github.com/ThatsTheEnd/lvpm/commit/c98e2a3032bd15ce61beab8704ab81ab1a7b80c8))
* **s8:** federated community index ([#122](https://github.com/ThatsTheEnd/lvpm/issues/122)) ([c941b4e](https://github.com/ThatsTheEnd/lvpm/commit/c941b4e2617d8bea2b96e6c74dd718cf1bdebb3f))
* **s9:** application builds and SBOM ([#119](https://github.com/ThatsTheEnd/lvpm/issues/119)) ([2db838e](https://github.com/ThatsTheEnd/lvpm/commit/2db838e4582c25e85542841795c58c93220d0e1b))

## [0.24.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.23.0...v0.24.0) (2026-11-04)


### Features

* **s12:** install hooks off by default ([#118](https://github.com/ThatsTheEnd/lvpm/issues/118)) ([efffcdc](https://github.com/ThatsTheEnd/lvpm/commit/efffcdc647b30c2fd4f09a08238f1d39ab937055))
* **s7c:** pin a dependency to one source ([#116](https://github.com/ThatsTheEnd/lvpm/issues/116)) ([aab9a58](https://github.com/ThatsTheEnd/lvpm/commit/aab9a58efa15ef42063b7130358e9c0d35b6c260))

## [0.23.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.22.0...v0.23.0) (2026-11-02)


### Features

* **s5b:** lvpm build runs the check ([#114](https://github.com/ThatsTheEnd/lvpm/issues/114)) ([7c53245](https://github.com/ThatsTheEnd/lvpm/commit/7c5324565bd6f0bcd47daef7841c8c387f0b7c48))

## [0.22.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.21.0...v0.22.0) (2026-11-02)


### Features

* **s6:** attestations and consumer policy ([#112](https://github.com/ThatsTheEnd/lvpm/issues/112)) ([5ee9d47](https://github.com/ThatsTheEnd/lvpm/commit/5ee9d47dfe216d226069902a7b2649ba93a71d36))

## [0.21.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.20.0...v0.21.0) (2026-11-02)


### Features

* **s5:** lvpm check ([#109](https://github.com/ThatsTheEnd/lvpm/issues/109)) ([690d939](https://github.com/ThatsTheEnd/lvpm/commit/690d9393dd4c440416a762f441bb39e60f7832d4))

## [0.20.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.19.0...v0.20.0) (2026-11-02)


### Features

* **s10d:** --yes, --labview-bitness, no lvpm update ([#106](https://github.com/ThatsTheEnd/lvpm/issues/106)) ([3b51c59](https://github.com/ThatsTheEnd/lvpm/commit/3b51c59d472dfa4f6e3002fb79861e0cd1772958))

## [0.19.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.18.0...v0.19.0) (2026-11-01)


### Features

* **s11:** dependency groups ([#104](https://github.com/ThatsTheEnd/lvpm/issues/104)) ([29eef60](https://github.com/ThatsTheEnd/lvpm/commit/29eef607a1493e9b155e6f9bb97c32e105dce0ff))

## [0.18.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.17.0...v0.18.0) (2026-11-01)


### Features

* [sources] min-age-days holds back too-new package versions ([#101](https://github.com/ThatsTheEnd/lvpm/issues/101)) ([e08e2ab](https://github.com/ThatsTheEnd/lvpm/commit/e08e2ab6e0199c2f6267f121088d4d995dd56ba1))

## [0.17.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.16.0...v0.17.0) (2026-11-01)


### Features

* **s4:** GitHub Releases as a feed ([#98](https://github.com/ThatsTheEnd/lvpm/issues/98)) ([a3c376c](https://github.com/ThatsTheEnd/lvpm/commit/a3c376c33333cf8c75280aa20165cce4b1895051))

## [0.16.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.15.0...v0.16.0) (2026-10-31)


### Features

* **tui:** ship the TUI as `lvpm tui`, fix its rough edges, and have init write a .gitignore ([#96](https://github.com/ThatsTheEnd/lvpm/issues/96)) ([994026b](https://github.com/ThatsTheEnd/lvpm/commit/994026be672b2adaa9010f9b5c3a63017cb4b0a4))

## [0.15.0](https://github.com/ThatsTheEnd/lvpm/compare/v0.14.1...v0.15.0) (2026-10-31)


### Features

* **init:** lvpm init and init --from .vipc / vipm.toml ([#94](https://github.com/ThatsTheEnd/lvpm/issues/94)) ([a05b866](https://github.com/ThatsTheEnd/lvpm/commit/a05b866c3d1ba2b7dce5e0fa05ed77dae09fbb38))
