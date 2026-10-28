// 界面多语言：中文是源语言，其它语言按中文原文查表。
// UI translations: Chinese is the source language; other languages are looked up by
// the Chinese text.
//
// - 代码里的文字用 `t('中文 {name}', { name })`，参数用 `{名字}` 占位。
//   Strings in code use `t('中文 {name}', { name })` with `{name}` placeholders.
// - HTML 里的静态文字（文本节点与 title / placeholder / aria-label / alt）由
//   `translateDom` 按原文整体替换，切回中文时恢复原文。
//   Static HTML text (text nodes and title / placeholder / aria-label / alt) is replaced
//   as a whole by `translateDom`, and restored when switching back to Chinese.
// - 含排版的大段说明用 `data-i18n-html="键"`，按键整体替换 innerHTML。
//   Formatted blocks use `data-i18n-html="key"` and swap their whole innerHTML.
// - 查不到的文字原样显示（中文），所以漏翻不会出错，只是显示中文。
//   Missing entries fall back to the Chinese text, so a gap shows Chinese, never breaks.

export type Lang = 'zh-CN' | 'en';
/** 语言偏好（设置里保存的值）。Language preference (as stored in settings). */
export type LangPref = 'system' | Lang;

let lang: Lang = 'zh-CN';

/** 英文词典：中文原文 → 英文。English dictionary: Chinese source → English. */
const EN: Record<string, string> = {
  // ---- 导航与工具栏 Navigation and toolbar ----
  导航: 'Navigation',
  音效库: 'Library',
  手机遥控: 'Phone remote',
  使用说明: 'Help',
  设置: 'Settings',
  文件夹: 'Folder',
  新建文件夹: 'New folder',
  重新扫描文件夹: 'Rescan the folder',
  重新扫描: 'Rescan',
  在文件管理器中打开音效库文件夹: 'Open the library folder in the file manager',
  '在文件管理器中打开：{path}': 'Open in the file manager: {path}',
  全部音效: 'All sounds',
  搜索音效: 'Search sounds',
  停止所有正在播放的音效: 'Stop every playing sound',
  全部停止: 'Stop all',
  导入音效: 'Import',
  '{n} 个音效': '{n} sounds',

  // ---- 底部条 Bottom bar ----
  麦克风静音: 'Mute microphone',
  你的麦克风: 'Your mic',
  '麦克风音量（双击恢复 0 dB）': 'Mic volume (double-click for 0 dB)',
  '试听：音效同时在耳机 / 扬声器里播放': 'Monitor: also play effects on headphones / speakers',
  '混音发送到哪里（点击修改）': 'Where the mix is sent (click to change)',
  其他软件的麦克风选: 'Mic to pick in other apps',
  正在用扬声器输出: 'Outputting to speakers',
  '其他软件收不到，点此改为虚拟音频线': "Other apps can't hear it — switch to a virtual cable",
  '混音正在输出到「{device}」，只有你自己能听到。点击在设置里改为虚拟音频线（如 CABLE Input）。':
    'The mix is going to "{device}", so only you can hear it. Click to switch to a virtual cable (e.g. CABLE Input) in Settings.',
  '你的声音 + 音效输出到「{device}」；其他软件的麦克风选「{mic}」。':
    'Your voice + effects go to "{device}"; in other apps pick "{mic}" as the microphone.',
  '已静音，点击取消': 'Muted — click to unmute',
  点击静音: 'Click to mute',
  麦克风: 'Microphone',
  麦克风未连接: 'Microphone not connected',
  已静音: 'Muted',
  '等待麦克风连接…': 'Waiting for the microphone…',
  '试听中：音效同时在「{device}」播放（点击关闭）': 'Monitoring: effects also play on "{device}" (click to turn off)',
  '试听已开启，但没有生效：{note}': "Monitor is on but not active: {note}",
  '试听已关闭：点击开启，音效同时在耳机 / 扬声器播放': 'Monitor is off: click to also play effects on headphones / speakers',

  // ---- 拖入与导入 Drop and import ----
  松开即可导入音效: 'Drop to import',
  '松开即可导入到「{folder}」': 'Drop to import into "{folder}"',
  '支持 MP3 / WAV / FLAC / OGG / M4A / AAC': 'Supports MP3 / WAV / FLAC / OGG / M4A / AAC',
  '「{name}」导入失败：{error}': 'Importing "{name}" failed: {error}',
  '已导入 {n} 个音效到{where}': 'Imported {n} sound(s) into {where}',
  '「{folder}」': '"{folder}"',

  // ---- 网格与卡片 Grid and cards ----
  '没有找到「{query}」': 'Nothing matches "{query}"',
  '「{folder}」还没有音效': '"{folder}" has no sounds yet',
  还没有音效: 'No sounds yet',
  '把音频文件拖进窗口，或点击下方按钮导入': 'Drag audio files into the window, or import with the button below',
  '编辑 {name}': 'Edit {name}',
  '循环播放（再按一下卡片停止）\n点击改为单次': 'Loop (press the card again to stop)\nClick for play once',
  '单次播放\n点击改为循环': 'Play once\nClick for loop',
  '独占：播放时打断其它音效\n点击改为叠加': 'Exclusive: stops other sounds when it plays\nClick for overlap',
  '叠加：可与其它音效同时播放\n点击改为独占': 'Overlap: plays alongside other sounds\nClick for exclusive',
  '音量：点击后上下拖动': 'Volume: click, then drag up / down',
  '倍速（变速同时变调）：点击切换，右键反向': 'Speed (changes pitch too): click to cycle, right-click to go back',
  拖动跳转: 'Drag to seek',
  '{name} 音量': '{name} volume',
  '保存失败：{error}': 'Save failed: {error}',

  // ---- 文件夹 Folders ----
  '「{name}」更多操作': 'More actions for "{name}"',
  新建子文件夹: 'New subfolder',
  重命名: 'Rename',
  删除: 'Delete',
  全部: 'All',
  '在「{parent}」里新建': 'Create inside "{parent}"',
  在音效库根目录新建: 'Create at the library root',
  '新建失败：{error}': 'Create failed: {error}',
  重命名文件夹: 'Rename folder',
  '当前：{path}': 'Current: {path}',
  '重命名失败：{error}': 'Rename failed: {error}',
  删除文件夹: 'Delete folder',
  '确定删除「{path}」？只能删除没有音效的文件夹，删除的文件夹会移到回收站。':
    'Delete "{path}"? Only folders without sounds can be deleted; it goes to the recycle bin.',
  '删除失败：{error}': 'Delete failed: {error}',

  // ---- 编辑对话框 Edit dialog ----
  编辑音效: 'Edit sound',
  关闭: 'Close',
  '名称（即文件名）': 'Name (the file name)',
  播放方式: 'Playback',
  循环播放: 'Loop',
  '再按一下停止；关闭则每次播一遍': 'Press again to stop; off = play once each time',
  独占: 'Exclusive',
  '播放时打断其它音效；关闭则可叠加': 'Stops other sounds when it plays; off = overlap',
  音量: 'Volume',
  倍速: 'Speed',
  '变速同时变调，双击滑块恢复 1.0×': 'Speed changes pitch too; double-click the slider for 1.0×',
  快捷键: 'Hotkey',
  清除: 'Clear',
  '点击后按下组合键（其他程序全屏时也能触发），Esc 取消': 'Click, then press a key combo (works while other programs are full-screen); Esc cancels',
  取消: 'Cancel',
  保存: 'Save',
  确定: 'OK',
  '请按下组合键…': 'Press a key combo…',
  点击设置快捷键: 'Click to set a hotkey',
  '单个字母或数字会影响打字，请加上 Ctrl / Alt / Shift': 'A lone letter or digit would block typing; add Ctrl / Alt / Shift',
  '（根目录）': '(root)',
  '文件：{file}': 'File: {file}',
  '文件：{file}（{error}）': 'File: {file} ({error})',
  '再点一次：移到回收站': 'Click again: move to recycle bin',
  '已删除「{name}」（在回收站可恢复）': 'Deleted "{name}" (restorable from the recycle bin)',

  // ---- 设置 Settings ----
  外观: 'Appearance',
  跟随系统: 'System',
  深色: 'Dark',
  浅色: 'Light',
  自定义颜色: 'Custom colour',
  '主题色用于按钮、播放中的高亮与进度条。': 'The accent colour is used for buttons, the playing highlight and progress bars.',
  '任务栏 / 托盘图标也跟随主题色': 'Taskbar / tray icon follows the accent colour too',
  紫: 'Violet',
  蓝: 'Blue',
  青: 'Teal',
  绿: 'Green',
  橙: 'Orange',
  粉: 'Pink',
  红: 'Red',
  '保存外观失败：{error}': 'Saving the appearance failed: {error}',
  音效库位置: 'Library location',
  '浏览…': 'Browse…',
  把当前音效库里的文件一起移动过去: 'Move the current library files there too',
  打开文件夹: 'Open folder',
  应用: 'Apply',
  '音效库就是这个文件夹：子文件夹即分类，在资源管理器里放进去的音频文件，点左侧「刷新」后出现。':
    'The library is this folder: subfolders are categories. Audio files you put there in the file manager appear after "Rescan" on the left.',
  '无法读取音效库：{error}': "Can't read the library: {error}",
  '无法打开文件夹选择框：{error}': "Can't open the folder picker: {error}",
  已移动音效库到新位置: 'Moved the library to the new location',
  已切换音效库位置: 'Switched the library location',
  '修改位置失败：{error}': 'Changing the location failed: {error}',
  已重新扫描音效库: 'Library rescanned',
  '扫描失败：{error}': 'Rescan failed: {error}',

  音频链路: 'Audio chain',
  '混音 + 音效': 'Mix + effects',
  虚拟麦克风: 'Virtual mic',
  '其他软件': 'Other apps',
  '你的麦克风（输入）': 'Your mic (input)',
  刷新设备列表: 'Refresh the device list',
  刷新: 'Refresh',
  '自动（优先系统默认的硬件麦克风）': 'Auto (prefer the default hardware mic)',
  '{name}（系统默认）': '{name} (system default)',
  '虚拟设备（一般不用作麦克风）': 'Virtual devices (not usually mics)',
  '{name}（未连接）': '{name} (not connected)',
  '正在使用：{device}（{rate} Hz）': 'In use: {device} ({rate} Hz)',
  '切换麦克风失败：{error}': 'Switching the microphone failed: {error}',
  '输出音频线路': 'Output line',
  '虚拟音频线（推荐，其他软件可当作麦克风）': 'Virtual cables (recommended — other apps can use it as a mic)',
  '扬声器 / 耳机（只有自己能听到）': 'Speakers / headphones (only you hear it)',
  '当前输出是扬声器：只有你自己能听到，其他软件收不到。请选择一条虚拟音频线（如 {cable}），没有的话需要先安装 VB-Cable。':
    "The output is a speaker: only you can hear it, other apps can't. Pick a virtual audio cable (e.g. {cable}); install VB-Cable first if you don't have one.",
  '其他软件的麦克风选 {mic}': 'In other apps, pick {mic} as the microphone',
  '已输出到「{device}」': 'Now outputting to "{device}"',
  '已切换到扬声器（其他软件收不到）': "Switched to speakers (other apps can't hear it)",
  '切换输出失败：{error}': 'Switching the output failed: {error}',
  '无法读取设备列表：{error}': "Can't read the device list: {error}",
  '自动创建虚拟麦克风（重启生效）': 'Create a virtual mic automatically (after restart)',
  '已创建：其他软件的麦克风选「{source}」': 'Created: pick "{source}" as the mic in other apps',
  '未创建：{note}': 'Not created: {note}',
  '未启用（已选择其它输出设备）': 'Off (another output device is selected)',
  '重启 SoundBridge 后生效': 'Takes effect after restarting SoundBridge',
  '试听（自己也听到音效）': 'Monitor (hear the effects yourself)',
  '音效同时在耳机 / 扬声器里播放': 'Also play effects on headphones / speakers',
  不包含你自己的声音: "Doesn't include your own voice",
  试听设备: 'Monitor device',
  '试听音量（双击恢复 0 dB）': 'Monitor volume (double-click for 0 dB)',
  系统默认播放设备: 'System default playback device',
  已关闭: 'Off',
  '正在试听：{device}': 'Monitoring on: {device}',
  '试听设置失败：{error}': 'Monitor setting failed: {error}',

  窗口: 'Window',
  点击关闭按钮时: 'When closing the window',
  每次询问: 'Ask every time',
  最小化到托盘: 'Minimize to tray',
  退出: 'Quit',
  '最小化到托盘后音效、快捷键、手机遥控照常可用；单击托盘图标重新打开。':
    'In the tray, sounds, hotkeys and the phone remote keep working; click the tray icon to reopen.',

  基础操作: 'Basics',
  '各平台接入其他软件': 'Connecting other apps on each platform',
  当前系统: 'This system',
  关于: 'About',
  '设置保存在：': 'Settings are saved in: ',

  // ---- 新音效的默认设置 Defaults for new sounds ----
  新音效的默认设置: 'Defaults for new sounds',
  关闭则每次播一遍: 'Off = play once each time',
  应用到所有现有音效: 'Apply to all existing sounds',
  '导入或放进音效库文件夹的新音效会使用这些设置；已有音效不会自动改变。':
    "New sounds (imported or put into the library folder) start with these settings; existing sounds don't change automatically.",
  '把这些默认设置应用到全部 {n} 个音效？快捷键不受影响。': 'Apply these defaults to all {n} sounds? Hotkeys are not affected.',
  '已应用到 {n} 个音效': 'Applied to {n} sounds',
  '应用失败：{error}': 'Apply failed: {error}',

  // ---- 音效资源 Sound sites ----
  音效资源: 'Sound sites',
  推荐网站: 'Recommended',
  我添加的: 'My sites',
  '还没有添加网址。点右上角「添加网址」，把常用的音效网站收藏到这里。':
    'No sites yet. Click "Add site" at the top right to keep your favourite sound sites here.',
  '{n} 个网站': '{n} sites',
  预览: 'Preview',
  打开: 'Open',
  在浏览器中打开: 'Open in browser',
  编辑: 'Edit',
  添加网址: 'Add site',
  编辑网址: 'Edit site',
  '网址（如 freesound.org）': 'Address (e.g. freesound.org)',
  下一步: 'Next',
  网站名称: 'Site name',
  删除网站: 'Remove site',
  '删除「{name}」？': 'Remove "{name}"?',
  '点「预览」在 App 窗口里打开网站并试听；在预览窗口里下载的音频文件会自动存进音效库的「下载」文件夹。':
    'Click "Preview" to open a site in an app window and listen there; audio files you download in that window are saved straight into the "Downloads" folder of your library.',
  '最大的开放音效社区之一，数十万条用户上传的音效，可在线试听。':
    'One of the largest open sound communities: hundreds of thousands of user uploads you can preview online.',
  'CC0 / CC BY 等，下载需免费注册': 'CC0 / CC BY etc.; free sign-up to download',
  '大量免费音效，分类清晰，可在线试听。': 'Lots of free sound effects, clearly categorised, with online preview.',
  'Pixabay 许可：免费可商用，无需署名': 'Pixabay licence: free for commercial use, no attribution',
  '精选免费音效，质量整齐，免注册直接下载。': 'Curated free sound effects of consistent quality; download without signing up.',
  'Mixkit 免费许可': 'Mixkit free licence',
  '十几万条专业音效，分类细致。': 'Well over a hundred thousand professional sound effects, finely categorised.',
  '下载需注册，免费账号需署名': 'Sign-up required; free accounts must give attribution',
  '老牌免费音效站，常见日常音效齐全。': 'A long-running free site with all the common everyday sounds.',
  'CC BY / 公有领域等，逐条标注': 'CC BY / public domain etc., marked per sound',
  'BBC 公开的音效档案，环境声、拟音丰富。': "The BBC's public sound archive, rich in ambience and Foley.",
  'RemArc 许可：个人、教育、研究用途免费': 'RemArc licence: free for personal, educational and research use',
  '开源素材社区，含大量音效与音乐。': 'An open-source asset community with many sound effects and music.',
  'CC0 / CC BY / GPL 等，逐条标注': 'CC0 / CC BY / GPL etc., marked per item',
  '每年免费发布的大型专业音效合集（数十 GB）。': 'A large professional sound bundle released free every year (tens of GB).',
  '免版税，可商用': 'Royalty-free, commercial use allowed',
  '热门梗音效按钮，点一下就能试听。': 'Popular meme sound buttons — click to listen.',
  '用户上传，版权不明，建议仅自用': 'User uploads with unclear rights; best for personal use',
  '中文音效素材站，分类多、搜索方便。': 'A Chinese sound library with many categories and easy search.',
  '免费与付费混合，按单个素材的授权使用': 'Mix of free and paid; follow each item\'s licence',
  '免费音效与按钮音，免注册下载。': 'Free sound effects and button sounds, no sign-up needed.',
  '免费使用，需遵守网站条款': "Free to use under the site's terms",

  // ---- 手机遥控 Phone remote ----
  '允许同一 Wi-Fi 下的手机打开界面': 'Let phones on the same Wi-Fi open the interface',
  手机扫码打开: 'Scan with your phone',
  用手机相机或微信扫一扫: 'Scan with the phone camera',
  重新生成访问码: 'New access code',
  '开启后显示二维码。网址带有访问码，没扫过码的设备打不开；可随时关闭。':
    "Turn it on to show a QR code. The link carries an access code, so devices that haven't scanned it can't open it. Turn it off any time.",
  '手机遥控：已开启（点击查看二维码）': 'Phone remote: on (click for the QR code)',
  '手机遥控：已关闭（点击开启）': 'Phone remote: off (click to turn on)',
  '手机遥控：{error}': 'Phone remote: {error}',

  // ---- 关闭询问 Close prompt ----
  '关闭 SoundBridge？': 'Close SoundBridge?',
  '最小化到托盘后，音效、快捷键和手机遥控照常可用，单击托盘图标重新打开。':
    'In the tray, sounds, hotkeys and the phone remote keep working; click the tray icon to reopen.',
  '记住我的选择（可在设置 → 窗口里修改）': 'Remember my choice (change it in Settings → Window)',
};

/** 含排版的大段说明（`data-i18n-html`）。Formatted blocks (`data-i18n-html`). */
const EN_HTML: Record<string, string> = {
  'help-basic': `
    <li><b>Import</b>: drag audio files into the window (or onto a folder on the left), or click "Import" at the top right. Files are saved into the library folder.</li>
    <li><b>Play</b>: click a card. While it plays, drag the progress bar at the bottom of the card to seek; click the round button to stop; "Stop all" is at the top right.</li>
    <li><b>Card buttons</b>: the loop icon switches loop / play once (press a looping card again to stop); the exclusive icon (one dot) / overlap icon (two circles) decides whether it stops other sounds; volume opens a vertical slider; speed steps through presets (right-click goes back).</li>
    <li><b>Hotkeys</b>: open the editor with "⋯" on the card, click the hotkey box and press a key combo; it works while other programs are full-screen too.</li>
    <li><b>Folders</b>: create and rename folders on the left; drag a card onto a folder to move it. After changing files in the file manager, click "Rescan".</li>
    <li><b>Your mic</b>: mute, adjust and watch the level at the bottom left; pick the device under "Audio chain" here (by default a hardware mic is picked automatically).</li>
    <li><b>Output line</b>: the bottom right shows which mic to pick in other apps; choose the virtual cable under "Audio chain" here.</li>
    <li><b>Monitor</b>: once the mix goes to a virtual cable you can't hear it yourself; turn on the headphones button at the bottom and the effects also play on your headphones (without your own voice).</li>
    <li><b>Phone remote</b>: the phone button on the left turns it on or off; scan the QR code to open the same interface on your phone.</li>
    <li><b>Sound sites</b>: the globe button on the left lists free / open sound websites, and you can add your own; in the app, "Preview" opens a site in a window, and audio you download there is saved straight into the library's "Downloads" folder.</li>`,
  'help-windows': `
    <li>Install a virtual audio cable such as VB-Cable (free).</li>
    <li>Under "Audio chain → Output line" here, pick <code>CABLE Input</code>.</li>
    <li>In other apps, pick <code>CABLE Output</code> as the microphone (shown at the bottom right).</li>
    <li>Turn on "Monitor" to hear your effects on your headphones.</li>
    <li>Keep the default playback device in Windows "Sound settings" on your headphones or speakers, not CABLE Input — otherwise everything the computer plays leaks into the mic.</li>`,
  'help-linux': `
    <li>Requires PipeWire (with pipewire-pulse) or PulseAudio, plus the <code>pactl</code> command (pulseaudio-utils).</li>
    <li>SoundBridge creates the virtual mic <code>SoundBridge-Microphone</code> at start-up and removes it on exit.</li>
    <li>In other apps, pick <code>SoundBridge-Microphone</code> as the microphone.</li>
    <li>Global hotkeys need an X11 session (on Wayland, switch to X11, or trigger sounds from the window / phone remote).</li>`,
  'help-macos': `
    <li>Install a virtual audio device such as BlackHole 2ch.</li>
    <li>Under "Audio chain → Output line" here, pick <code>BlackHole 2ch</code>.</li>
    <li>In other apps, pick <code>BlackHole 2ch</code> as the microphone. On first use, allow microphone access under "Privacy &amp; Security".</li>`,
};

/** 当前语言。The current language. */
export function currentLang(): Lang {
  return lang;
}

/** 翻译一段文字；`{名字}` 用参数替换。Translate a string; `{name}` placeholders are filled. */
export function t(zh: string, params?: Record<string, string | number>): string {
  let s = lang === 'en' ? (EN[zh] ?? zh) : zh;
  if (params) s = s.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
  return s;
}

const ATTRS = ['title', 'placeholder', 'aria-label', 'alt'] as const;
/** 文本节点的原文。Original text of text nodes. */
const origText = new WeakMap<Text, string>();
/** 元素属性的原文。Original attribute values of elements. */
const origAttr = new WeakMap<Element, Record<string, string>>();
/** 大段说明的原 HTML。Original HTML of formatted blocks. */
const origHtml = new WeakMap<Element, string>();

/** 翻译一个原文（保留首尾空白）。Translate one source string (keeping outer whitespace). */
function swap(source: string): string {
  const core = source.trim();
  if (!core) return source;
  const out = t(core);
  return out === core ? source : source.replace(core, out);
}

/** 把页面里的静态文字切到当前语言。Switch the page's static text to the current language. */
export function translateDom(root: ParentNode = document.body): void {
  root.querySelectorAll<HTMLElement>('[data-i18n-html]').forEach((e) => {
    if (!origHtml.has(e)) origHtml.set(e, e.innerHTML);
    const en = EN_HTML[e.dataset.i18nHtml ?? ''];
    e.innerHTML = lang === 'en' && en ? en : origHtml.get(e)!;
  });

  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      n.parentElement?.closest('script, style, [data-i18n-html], [data-i18n-skip]')
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  for (let n = walker.nextNode() as Text | null; n; n = walker.nextNode() as Text | null) {
    // 只处理含中文的原文，避免改动动态写入的英文 / 设备名。
    // Only Chinese source text, so dynamic English text / device names are left alone.
    if (!origText.has(n)) {
      if (!/[一-鿿]/.test(n.data)) continue;
      origText.set(n, n.data);
    }
    n.data = swap(origText.get(n)!);
  }

  root.querySelectorAll('*').forEach((e) => {
    if (e.closest('[data-i18n-skip]')) return;
    let saved = origAttr.get(e);
    for (const a of ATTRS) {
      const v = e.getAttribute(a);
      if (v === null) continue;
      if (!saved?.[a]) {
        if (!/[一-鿿]/.test(v)) continue;
        saved = { ...(saved ?? {}), [a]: v };
        origAttr.set(e, saved);
      }
      e.setAttribute(a, swap(saved[a]));
    }
  });
}

/** 设置当前语言并翻译页面。Set the language and translate the page. */
export function setLang(next: Lang): void {
  lang = next;
  document.documentElement.lang = next;
  try {
    localStorage.setItem('sb-lang', next);
  } catch {
    /* 存储不可用时忽略。Ignore when storage is unavailable. */
  }
  translateDom();
}

/** 启动时先用上次的语言，避免中英闪烁。Use the last language at start to avoid a flash. */
export function initLang(): void {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem('sb-lang');
  } catch {
    /* 同上。Same as above. */
  }
  const guess: Lang = navigator.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en';
  setLang(saved === 'en' || saved === 'zh-CN' ? saved : guess);
}
