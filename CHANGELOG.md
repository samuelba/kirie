# Changelog

## [0.3.0](https://github.com/samuelba/kirie/compare/v0.2.0...v0.3.0) (2026-11-23)


### Features

* **annotate:** add a speech bubble tool (@samuelba) ([5ac023d](https://github.com/samuelba/kirie/commit/5ac023dbe14b33182e0f6085d5306c27c8cda97d))
* **capture:** open captures taken without the overlay in the editor (@samuelba) ([50cf010](https://github.com/samuelba/kirie/commit/50cf010a05a6e6dac54caee63c56bf6959dbe029))
* **editor:** set the mark actions apart in a Selection section (@samuelba) ([a5c7871](https://github.com/samuelba/kirie/commit/a5c7871ebb9589c04b47595e257beeb83bb00d18))
* **overlay:** let M toggle the magnifier while picking a region (@samuelba) ([3747f09](https://github.com/samuelba/kirie/commit/3747f09f2a63503fd4cbc58440d408fe7125a8d9))
* **update:** check for and install updates from the app (@samuelba) ([500aa39](https://github.com/samuelba/kirie/commit/500aa394cdd6e3ebcbfe27951f1dc2e0db0e8bec))


### Bug Fixes

* **editor:** keep the selection outline visible on red marks (@samuelba) ([099f592](https://github.com/samuelba/kirie/commit/099f5927f4b67e54ec5a43cd0c11822834774fa1))
* **overlay:** soften the capture flash (@samuelba) ([fb54531](https://github.com/samuelba/kirie/commit/fb54531a7a40e837073effa432a6f07ba68afe03))

## [0.2.0](https://github.com/samuelba/kirie/compare/v0.1.1...v0.2.0) (2026-11-23)


### Features

* bring picked marks to the front or send them to the back (@samuelba) ([ea81909](https://github.com/samuelba/kirie/commit/ea81909f58ddc0936817470087121b2a1f43aa3c))
* **capture:** take a region, the topmost window or every screen without the overlay, to a file, stdout or the clipboard (@samuelba) ([ff31471](https://github.com/samuelba/kirie/commit/ff31471437c4058b0dcfb62ff8534721a5ba4b26))
* **capture:** wait before a screenshot with --delay, or from the tray (@samuelba) ([1611fcd](https://github.com/samuelba/kirie/commit/1611fcdafb6a9c17e637bf3a8f6234cb31174937))
* copy, cut, paste and duplicate marks (@samuelba) ([a343db1](https://github.com/samuelba/kirie/commit/a343db16ad9f001378479f802dd4c433f5df513d))
* **editor:** add a button that discards the image (@samuelba) ([e91b74e](https://github.com/samuelba/kirie/commit/e91b74e5f890cad9182ca013d7cb7ff441bea1fb))
* **editor:** add an about card, opened from Kirie's name in the header (@samuelba) ([a568d17](https://github.com/samuelba/kirie/commit/a568d1703c72826b5c84df821b596a89fd22b3b6))
* **editor:** close the editor with Ctrl+Q (@samuelba) ([bf67dfb](https://github.com/samuelba/kirie/commit/bf67dfbdc814b8aa0c5a17b0255642af4b310648))
* **editor:** paste a picture over the image as a mark (@samuelba) ([2391d34](https://github.com/samuelba/kirie/commit/2391d34606183132bd523fa4731e3e6c3f2d3d77))
* open a saved screenshot or show it in Files from the notification (@samuelba) ([bd00045](https://github.com/samuelba/kirie/commit/bd0004553e687fec935592dff9cbfb5c14703647))
* **overlay:** flash the screens as a screenshot is taken (@samuelba) ([ac35b13](https://github.com/samuelba/kirie/commit/ac35b139a9baee8e97966b64fe778966447cde18))
* **overlay:** magnify the pixels around the pointer while picking a region (@samuelba) ([ccbd6e1](https://github.com/samuelba/kirie/commit/ccbd6e1ceff71fd98bafa3500a74bbad905be9a8))
* **overlay:** tell when only one of several screens can be shown (@samuelba) ([9a1e1e4](https://github.com/samuelba/kirie/commit/9a1e1e493767c5c2b9f0edb1fcbfea610b30e559))
* **packaging:** register .kirie projects as a file type that Kirie opens (@samuelba) ([c64af33](https://github.com/samuelba/kirie/commit/c64af3388656fae36869dc6172171a943fbae023))
* pick several marks with a box or Shift+click, and move, restyle or delete them together (@samuelba) ([b019170](https://github.com/samuelba/kirie/commit/b019170d154169fcf65e0e04e57014f81d43fab2))
* pick translucent colors with an alpha strip in the color picker (@samuelba) ([74fdc23](https://github.com/samuelba/kirie/commit/74fdc23fecfab078d9f6c9131a178c1aaa77685c))
* save and copy in one go (@samuelba) ([030b64e](https://github.com/samuelba/kirie/commit/030b64e5e87bb28c2b6b8f7cee20c9a4e3e28824))
* save as, in a folder and format of your choice, from the editor and the overlay (@samuelba) ([c9bf18e](https://github.com/samuelba/kirie/commit/c9bf18e1c1ee354df14e5656c4c9942a69d56d8e))
* settings for the save folder, the file name and copying on save (@samuelba) ([6b4166e](https://github.com/samuelba/kirie/commit/6b4166e1ea60317ca6267f91e2d7f8a46b0cda29))
* show a tray icon, with a menu that opens Kirie or takes a screenshot (@samuelba) ([75a1960](https://github.com/samuelba/kirie/commit/75a1960745cad7601c2ea17449e620434667326b))
* take a color from the screenshot with the color picker's pipette (@samuelba) ([988a514](https://github.com/samuelba/kirie/commit/988a5144d5e7db9a530b1ce4e1420418a442fc64))


### Bug Fixes

* bring the editor to the front when the tray or a launcher opens it (@samuelba) ([88a56ab](https://github.com/samuelba/kirie/commit/88a56ab805d51e028cc3caca35a21582e1f37596))
* **editor:** drop the hard shadow that showed as a grey bar under the image (@samuelba) ([4599ec0](https://github.com/samuelba/kirie/commit/4599ec0feac3e3b3a3a8d7db00d1010e935b12a5))
* **editor:** make the zoom list wide enough for "Fit 100%" (@samuelba) ([2f107af](https://github.com/samuelba/kirie/commit/2f107afa22915b91708ff8308a8ad2ca961a95ad))
* **editor:** open images in place, and keep old data devices instead of releasing them (@samuelba) ([18750fc](https://github.com/samuelba/kirie/commit/18750fc4109817f9293ceacb78a9dd101be1b2e5))
* **editor:** take drops on GNOME, and paste through the window's own data device (@samuelba) ([75f8bb1](https://github.com/samuelba/kirie/commit/75f8bb1b2bb45f1435471a087a5b554f230d841a))
* guard pasted marks and image marks against hostile input, and keep copied marks in Kirie (@samuelba) ([ce38f2e](https://github.com/samuelba/kirie/commit/ce38f2ecdac8c77c0d3674f33b5a0a6b2a567191))
* wait for the clipboard service to say it holds the clipboard, and tighten Esc, restacking keys and pastes (@samuelba) ([9df1c91](https://github.com/samuelba/kirie/commit/9df1c91432c4edca99a4608c7447b3648c395422))


### Performance Improvements

* **capture:** encode GNOME Shell screenshots in parallel strips (@samuelba) ([28c9105](https://github.com/samuelba/kirie/commit/28c91052abb3346e13c3229125417d21982d6eec))

## [0.1.1](https://github.com/samuelba/kirie/compare/v0.1.0...v0.1.1) (2026-11-22)


### Bug Fixes

* **update:** trust the release signing key (@samuelba) ([3c4f7eb](https://github.com/samuelba/kirie/commit/3c4f7eb771471ee4b64fd033678390b652683617))

## 0.1.0 (2026-11-22)


### Features

* add the kirie app icon (@samuelba) ([e05d362](https://github.com/samuelba/kirie/commit/e05d3626abace6ac9446b17d863de6118b33ea7f))
* **annotate:** add documents, clean, sketch and ink styles, and the renderer (@samuelba) ([fb88a1d](https://github.com/samuelba/kirie/commit/fb88a1d221ff93a00328699e127e57865b15e4c5))
* **annotate:** add drawing tools and moving marks between coordinate spaces (@samuelba) ([542ded5](https://github.com/samuelba/kirie/commit/542ded541eb495928e52d9d3d593541573b78407))
* **annotate:** frame the saved picture, with room, a background and a shadow (@samuelba) ([23c436a](https://github.com/samuelba/kirie/commit/23c436a2584a34bd34575a34d111771b61e770cc))
* **annotate:** light an ellipse with the spotlight (@samuelba) ([1a4c32c](https://github.com/samuelba/kirie/commit/1a4c32c0e6277f592ab46e641a9399348866e71f))
* **annotate:** offer the same colours in every style (@samuelba) ([29673ba](https://github.com/samuelba/kirie/commit/29673ba478d47bc08c7ee05c716ad46bd387a930))
* **annotate:** resize marks with grips and turn boxes and ellipses (@samuelba) ([ecf6dc5](https://github.com/samuelba/kirie/commit/ecf6dc50f41e9e3e9c015f0dc1c1b5348a716db2))
* **annotate:** save and load editable .kirie projects (@samuelba) ([53641ff](https://github.com/samuelba/kirie/commit/53641ff5bd4a1c5fdb94e70b931c9b9c828825f9))
* **annotate:** set any stroke weight with a slider (@samuelba) ([592cf1d](https://github.com/samuelba/kirie/commit/592cf1dc656382b6a46f0a218aa1ebcf260eb3bc))
* **annotate:** set how bright spotlights leave the rest (@samuelba) ([f0a1f94](https://github.com/samuelba/kirie/commit/f0a1f9483fb964f852e82ec0fb638623f264fc1a))
* **annotate:** set the size, font and alignment of text, and its outline (@samuelba) ([72831d9](https://github.com/samuelba/kirie/commit/72831d94343b8c0e402b6829253d95a5732b97f5))
* **annotate:** shade the standard colors per style (@samuelba) ([9a54c5b](https://github.com/samuelba/kirie/commit/9a54c5b90c995c728d7c843cdc0739d4a7d495ab))
* **annotate:** show what a press on a mark will do with the cursor (@samuelba) ([714a20f](https://github.com/samuelba/kirie/commit/714a20fd1bed17425f833c733bc86afb21998d48))
* **annotate:** turn the clean style's shadow off (@samuelba) ([9332f29](https://github.com/samuelba/kirie/commit/9332f29929230b6b9bf8e11249ee10f1779963f9))
* antialias the overlay and editor, and draw the highlighter as a smooth even band (@samuelba) ([0f741c3](https://github.com/samuelba/kirie/commit/0f741c3678937cf7f5320c35d8c6e8418e9e39f9))
* **capture:** capture every screen through ext-image-copy-capture or wlr-screencopy (@samuelba) ([ff2a020](https://github.com/samuelba/kirie/commit/ff2a0203b27b0570200ddfb3ce8fa7466dd68a01))
* **capture:** fall back to the screenshot portal (@samuelba) ([a9a9509](https://github.com/samuelba/kirie/commit/a9a9509e38a6ff5543ff19150e60540d2e41cc7a))
* **capture:** pick windows on COSMIC (@samuelba) ([b7b3d2e](https://github.com/samuelba/kirie/commit/b7b3d2e21abae96618527c5a80a3985c06e06512))
* change the words of placed text, over as many lines as needed (@samuelba) ([33fb33e](https://github.com/samuelba/kirie/commit/33fb33e1b835f1fed2ae371180b01571d60f2b3a))
* **editor:** add KIRIE_EDITOR_DEMO for screenshots of the editor (@samuelba) ([5b33b46](https://github.com/samuelba/kirie/commit/5b33b46e1300a49a699a5fdbf6578551e2539760))
* **editor:** ask to save changes before closing (@samuelba) ([a11054c](https://github.com/samuelba/kirie/commit/a11054c661fb87370ac18816b65baa2c58e8baaf))
* **editor:** crop the image to the part worth keeping (@samuelba) ([abe9fb0](https://github.com/samuelba/kirie/commit/abe9fb01548daf26bcc936aeaa9c83e1a2c7b0c3))
* **editor:** follow the desktop's light or dark preference (@samuelba) ([7908574](https://github.com/samuelba/kirie/commit/7908574eea961d6de1dd25f033dc9247e5d12e2c))
* **editor:** mark up images in a window with zoom, paste, copy, save and projects (@samuelba) ([8ddcc9b](https://github.com/samuelba/kirie/commit/8ddcc9b6e28148e6a5f4063966ea798a389cb3ee))
* **editor:** open images and projects with the desktop's file dialog (@samuelba) ([e00e60d](https://github.com/samuelba/kirie/commit/e00e60dd2aa3500b80aa15b8b5e9fcb4f2c000c1))
* **editor:** open images dropped on the window (@samuelba) ([b118476](https://github.com/samuelba/kirie/commit/b118476fa10180a27155166e5fcb45c810b47432))
* **editor:** show the tool, pointer position, zoom and size in a status bar (@samuelba) ([bc8866c](https://github.com/samuelba/kirie/commit/bc8866c2ee5d488ce53e35bef6026227acc4c233))
* **editor:** zoom from the status bar (@samuelba) ([fed0ae9](https://github.com/samuelba/kirie/commit/fed0ae9a523f09df9c27239adba7c9a3fdb3ddc0))
* **gnome:** add the GNOME Shell extension for shortcut, capture, clipboard and overlay placement (@samuelba) ([8ac387f](https://github.com/samuelba/kirie/commit/8ac387f710fdc4b859c68c9eb88a83912604647a))
* hand captures to a single running editor, and capture from the editor (@samuelba) ([6596e9b](https://github.com/samuelba/kirie/commit/6596e9becde9d6e0eb764a11b75a3c24ad889a9e))
* keep the drawing options for the next screenshot and session (@samuelba) ([eb18cea](https://github.com/samuelba/kirie/commit/eb18cea179f56c4c3b83b65c325b00322a52dbd8))
* **overlay:** add a button that takes the whole screen (@samuelba) ([e296099](https://github.com/samuelba/kirie/commit/e2960990921e079c1ac0e8dce6fb6a61e946e790))
* **overlay:** drag the toolbar by its grip (@samuelba) ([4e685e9](https://github.com/samuelba/kirie/commit/4e685e99aaf156761dd65526d7692499388c8936))
* **overlay:** mark up the selection with a floating toolbar, then copy or save it (@samuelba) ([1cf06af](https://github.com/samuelba/kirie/commit/1cf06af1e468ad483b99c99f93832a6ddee91c9c))
* **overlay:** notify when a screenshot was copied or saved (@samuelba) ([f4d1ec4](https://github.com/samuelba/kirie/commit/f4d1ec4d7dc517b6e8c61882e282482bdfb1ec85))
* **overlay:** pick a region on frozen screens through layer-shell surfaces (@samuelba) ([7494dd4](https://github.com/samuelba/kirie/commit/7494dd463443b1d3864ca3c068b09334e96b02cb))
* **overlay:** pick a window by clicking it on GNOME, sway and Hyprland (@samuelba) ([3226b04](https://github.com/samuelba/kirie/commit/3226b046ac2b20d994cb7b195f70614e08204a82))
* **overlay:** remember a region for the next screenshot (@samuelba) ([1efaa47](https://github.com/samuelba/kirie/commit/1efaa47c4fd4fb52fa9dfda5c600ccb1ea4d5430))
* **overlay:** show the overlay in a fullscreen window where there is no layer-shell (@samuelba) ([6bed852](https://github.com/samuelba/kirie/commit/6bed85286276d124454a39c79035c17f85957277))
* **overlay:** show what Enter does on the toolbar's primary button (@samuelba) ([566290b](https://github.com/samuelba/kirie/commit/566290b240874c78773daf4eb2250bb4da8cde22))
* pick any colour, change the default swatches and keep custom ones (@samuelba) ([0e74987](https://github.com/samuelba/kirie/commit/0e7498712673b47ceec1c9c21500110921849d25))
* **scripts:** build a test bundle to try Kirie on a VM without a release (@samuelba) ([af7a962](https://github.com/samuelba/kirie/commit/af7a962ceb1d0f1572da24c93240b6908c0282c7))
* set up a screenshot shortcut on COSMIC, GNOME and other desktops (@samuelba) ([29d28d9](https://github.com/samuelba/kirie/commit/29d28d9df522a335e574df967aa2827f2f719b93))
* **ui:** draw blur as fading dots and pixelate as a mosaic (@samuelba) ([4330819](https://github.com/samuelba/kirie/commit/43308191c1f918dfcf6276c77efeafe859b2d8f3))
* **ui:** put the shadow toggle beside fill with a clearer icon (@samuelba) ([fc8cedc](https://github.com/samuelba/kirie/commit/fc8cedc62a604c8cb71ef056b8154d25716d0b3e))
* update kirie in place after checking the release's signature (@samuelba) ([c7bd855](https://github.com/samuelba/kirie/commit/c7bd855ebadf998966bc9c5277b9c2e8d7ba9371))


### Bug Fixes

* **annotate:** dim once around every spotlight, however many overlap (@samuelba) ([2aef2c4](https://github.com/samuelba/kirie/commit/2aef2c4f4d740f21b4f66d26993a6adc3a51df33))
* **annotate:** keep both passes of a sketched line within one stroke width (@samuelba) ([92030d2](https://github.com/samuelba/kirie/commit/92030d2858cce9df70e4ead125f24596bdee9f08))
* **annotate:** leave nothing to recover under blur, pixelate and redact (@samuelba) ([27505e2](https://github.com/samuelba/kirie/commit/27505e28354db7070321e3300f1c7ca49026af95))
* **annotate:** measure text in its face and size for its outline and hits (@samuelba) ([303f361](https://github.com/samuelba/kirie/commit/303f3618bf0842affc6a8c3f457f9d185ab2a8d8))
* **capture:** give up on a compositor that never answers, and use the stride of the chosen format (@samuelba) ([0e5b836](https://github.com/samuelba/kirie/commit/0e5b836131861873a77bcfb2cde176c19bb1de35))
* **editor:** come to the front after a screenshot taken from the editor (@samuelba) ([84ac6e7](https://github.com/samuelba/kirie/commit/84ac6e7478f3de2ad8d31568b04732093d092b7d))
* **editor:** draw marks above the image instead of under it (@samuelba) ([7de03cc](https://github.com/samuelba/kirie/commit/7de03cc2f916dc8b6176d165657605ba1320efe7))
* **editor:** show one divider before the theme button without an image (@samuelba) ([adefbd5](https://github.com/samuelba/kirie/commit/adefbd54d60562b759339832c01e43dbb2a7b128))
* **editor:** show the picture's shadow without a margin, and marks beyond it while drawing (@samuelba) ([6f2a9e9](https://github.com/samuelba/kirie/commit/6f2a9e92092745c8615ef0776d9f0b8772a4986f))
* keep gestures with what they started, finish saves before Escape, and cache text outlines (@samuelba) ([1ec40d4](https://github.com/samuelba/kirie/commit/1ec40d4fddc4332236a55a89e586a9685772c154))
* **overlay:** fill the screen on GNOME 46 (@samuelba) ([df22646](https://github.com/samuelba/kirie/commit/df22646ebf5e6cef45ffde8488837d718826e696))
* **overlay:** keep the toolbar its height while dragging it (@samuelba) ([820937c](https://github.com/samuelba/kirie/commit/820937cbf7572e3287a4f70c44ab071fe7cf0f7c))
* **overlay:** take the overlay off the screen on GNOME 46 (@samuelba) ([777f997](https://github.com/samuelba/kirie/commit/777f997f61f52579cae213ac0f08f46be5e67e18))
* **packaging:** crop the empty margin around the app icon (@samuelba) ([185f820](https://github.com/samuelba/kirie/commit/185f820223006f1c68acb369d7b7a1e0968cfaae))
* **security:** close the extension's capture to free-form arguments and harden hand-offs and the installer (@samuelba) ([16a9f02](https://github.com/samuelba/kirie/commit/16a9f0253846020da27d299e050311ba988978a3))
* start Kirie from the app menu (@samuelba) ([f4edf11](https://github.com/samuelba/kirie/commit/f4edf1160f0bb5bded05952c033e9279d2cfbee2))
* take screenshots from an editor the overlay opened on GNOME (@samuelba) ([3145225](https://github.com/samuelba/kirie/commit/31452259be1418e5c2cca172f37742cf6364a9fe))
