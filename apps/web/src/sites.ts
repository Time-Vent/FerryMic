// 内置的免费 / 开放音效资源网站。描述与授权说明是中文原文，界面里经 t() 翻译。
// Built-in free / open sound sites. Descriptions and licence notes are Chinese source
// text, translated through t() in the UI.
//
// 授权只是概括，以各网站的说明为准。Licence notes are summaries; each site's own terms rule.

export interface Site {
  name: string;
  url: string;
  /** 一句话介绍（中文原文）。One-line description (Chinese source). */
  desc: string;
  /** 授权概括（中文原文）。Licence summary (Chinese source). */
  license: string;
}

export const BUILTIN_SITES: Site[] = [
  {
    name: 'Freesound',
    url: 'https://freesound.org/',
    desc: '最大的开放音效社区之一，数十万条用户上传的音效，可在线试听。',
    license: 'CC0 / CC BY 等，下载需免费注册',
  },
  {
    name: 'Pixabay',
    url: 'https://pixabay.com/sound-effects/',
    desc: '大量免费音效，分类清晰，可在线试听。',
    license: 'Pixabay 许可：免费可商用，无需署名',
  },
  {
    name: 'Mixkit',
    url: 'https://mixkit.co/free-sound-effects/',
    desc: '精选免费音效，质量整齐，免注册直接下载。',
    license: 'Mixkit 免费许可',
  },
  {
    name: 'ZapSplat',
    url: 'https://www.zapsplat.com/',
    desc: '十几万条专业音效，分类细致。',
    license: '下载需注册，免费账号需署名',
  },
  {
    name: 'SoundBible',
    url: 'https://soundbible.com/',
    desc: '老牌免费音效站，常见日常音效齐全。',
    license: 'CC BY / 公有领域等，逐条标注',
  },
  {
    name: 'BBC Sound Effects',
    url: 'https://sound-effects.bbcrewind.co.uk/',
    desc: 'BBC 公开的音效档案，环境声、拟音丰富。',
    license: 'RemArc 许可：个人、教育、研究用途免费',
  },
  {
    name: 'OpenGameArt',
    url: 'https://opengameart.org/',
    desc: '开源素材社区，含大量音效与音乐。',
    license: 'CC0 / CC BY / GPL 等，逐条标注',
  },
  {
    name: 'Sonniss',
    url: 'https://sonniss.com/gameaudiogdc',
    desc: '每年免费发布的大型专业音效合集（数十 GB）。',
    license: '免版税，可商用',
  },
  {
    name: 'Myinstants',
    url: 'https://www.myinstants.com/',
    desc: '热门梗音效按钮，点一下就能试听。',
    license: '用户上传，版权不明，建议仅自用',
  },
  {
    name: '爱给网',
    url: 'https://www.aigei.com/sound/',
    desc: '中文音效素材站，分类多、搜索方便。',
    license: '免费与付费混合，按单个素材的授权使用',
  },
  {
    name: 'SoundJay',
    url: 'https://www.soundjay.com/',
    desc: '免费音效与按钮音，免注册下载。',
    license: '免费使用，需遵守网站条款',
  },
];

/** 网址里的主机名（去掉 www.）。Host name of a URL (without www.). */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

/** 补全协议：没写 http(s):// 时加上 https://。Add https:// when no scheme is given. */
export function normalizeUrl(input: string): string {
  const s = input.trim();
  return /^https?:\/\//i.test(s) ? s : `https://${s}`;
}
