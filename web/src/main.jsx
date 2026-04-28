import React, { useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import {
  Activity,
  AlertTriangle,
  Check,
  Clock,
  FileJson,
  ListChecks,
  MessageCircle,
  Moon,
  Plus,
  RefreshCw,
  Save,
  Send,
  Settings,
  Sun,
  Trash2,
  Users,
  X,
} from 'lucide-react';
import './styles.css';

const emptyConfigPath = '-';
const UI_PREFS_KEY = 'welinker-ui-prefs';
const themes = [
  { id: 'calm', label: { en: 'Calm', zh: '静心' } },
  { id: 'lotus', label: { en: 'Lotus', zh: '莲雾' } },
  { id: 'ocean', label: { en: 'Ocean', zh: '海风' } },
  { id: 'amber', label: { en: 'Amber', zh: '暖橙' } },
  { id: 'berry', label: { en: 'Berry', zh: '莓果' } },
];

const copy = {
  en: {
    loading: 'Loading',
    readyLocal: 'Ready',
    offline: 'offline',
    identity: 'identity',
    identities: 'identities',
    wechatIdentity: 'WeChat identity',
    wechatIdentities: 'WeChat identities',
    identitiesReady: (count) => `${count} WeChat ${count === 1 ? 'identity' : 'identities'} ready`,
    identitiesAvailable: (count) => `${count} WeChat ${count === 1 ? 'identity' : 'identities'} available`,
    noIdentity: 'No identity',
    unknown: 'unknown',
    accountId: 'account_id',
    wechatId: 'WeChat ID',
    userUnknown: 'user unknown',
    defaultHome: 'default home',
    brandSubtitle: '',
    calmMode: 'Calm mode',
    language: 'Language',
    theme: 'Theme',
    light: 'Light',
    dark: 'Dark',
    compose: 'WeChat',
    assistant: 'Assistant',
    preferences: 'Preferences',
    today: '',
    heroTitle: 'Send',
    heroCopy: '',
    readyIdentities: 'Identities',
    currentIdentity: 'Current',
    pickIdentity: '',
    refresh: 'Refresh',
    identityEmpty: 'No WeChat identity is connected yet.',
    composeMessage: 'Compose message',
    composeDescription: '',
    from: 'From',
    to: 'Contact',
    required: '',
    contactHint: '',
    message: 'Message',
    characters: 'characters',
    messagePlaceholder: '',
    attachmentLink: 'Attachment link',
    optional: '',
    notSet: 'Not set',
    none: 'None',
    clear: 'Clear',
    sendMessage: 'Send message',
    inUse: 'In use',
    available: 'Available',
    draftCleared: 'Draft cleared',
    draftClearedDetail: 'The message area is fresh again',
    sending: 'Sending',
    sent: 'Sent',
    messageSent: 'Message sent',
    preparedFor: (to) => `Prepared for ${to}`,
    sendNeedsAttention: 'Send needs attention',
    assistantTitle: 'Assistant',
    assistantDescription: '',
    assistantName: 'Assistant name',
    thread: 'Thread',
    keepsContext: 'Keeps context',
    prompt: 'Prompt',
    new: 'New',
    send: 'Send',
    promptHint: '',
    promptPlaceholder: '',
    noChat: 'No local chat messages yet.',
    messageRequired: 'Message is required',
    ready: 'Ready',
    assistantReplied: 'Assistant replied',
    defaultAssistant: 'default assistant',
    you: 'You',
    assistantNeedsAttention: 'Assistant needs attention',
    preferencesDescription: '',
    notLoaded: 'Not loaded',
    edited: 'Edited',
    loaded: 'Loaded',
    saved: 'Saved',
    saving: 'Saving',
    savedRestart: 'Saved; restart when convenient',
    advancedPreferences: 'Advanced preferences',
    reload: 'Reload',
    format: 'Format',
    save: 'Save',
    formatted: 'Formatted',
    recentMoments: 'Recent moments',
    recentDescription: 'What changed during this visit',
    events: (count) => `${count} event${count === 1 ? '' : 's'}`,
    nothingYet: 'Nothing has happened yet.',
    atAGlance: 'At a glance',
    summaryDescription: 'Current workspace settings',
    savedAt: 'Saved at',
    defaultAssistantLabel: 'Default assistant',
    listeningOn: 'Listening on',
    assistants: 'Assistants',
    noAssistants: 'No assistants are configured.',
    workspaceRefreshed: 'Workspace refreshed',
    workspaceUnavailable: 'Workspace unavailable',
    preferencesLoaded: 'Preferences loaded',
    preferencesSaved: 'Preferences saved',
    preferencesUnavailable: 'Preferences unavailable',
    preferencesSaveFailed: 'Preferences save failed',
    settingsFile: 'settings file',
  },
  zh: {
    loading: '加载中',
    readyLocal: '就绪',
    offline: '离线',
    identity: '身份',
    identities: '身份',
    wechatIdentity: '微信身份',
    wechatIdentities: '微信身份',
    identitiesReady: (count) => `${count} 个身份`,
    identitiesAvailable: (count) => `${count} 个身份`,
    noIdentity: '未选择身份',
    unknown: '未知',
    accountId: 'account_id',
    wechatId: '微信 ID',
    userUnknown: '用户未知',
    defaultHome: '默认入口',
    brandSubtitle: '',
    calmMode: '静心模式',
    language: '语言',
    theme: '主题',
    light: '亮色',
    dark: '暗色',
    compose: '微信',
    assistant: '助手',
    preferences: '偏好',
    today: '',
    heroTitle: '发送',
    heroCopy: '',
    readyIdentities: '身份',
    currentIdentity: '当前',
    pickIdentity: '',
    refresh: '刷新',
    identityEmpty: '还没有连接微信身份。',
    composeMessage: '撰写消息',
    composeDescription: '',
    from: '发送身份',
    to: '联系人',
    required: '',
    contactHint: '',
    message: '消息',
    characters: '字符',
    messagePlaceholder: '',
    attachmentLink: '附件链接',
    optional: '',
    notSet: '未填写',
    none: '无',
    clear: '清空',
    sendMessage: '发送消息',
    inUse: '使用中',
    available: '可用',
    draftCleared: '草稿已清空',
    draftClearedDetail: '消息区域已重置',
    sending: '发送中',
    sent: '已发送',
    messageSent: '消息已发送',
    preparedFor: (to) => `已准备发送给 ${to}`,
    sendNeedsAttention: '发送需要处理',
    assistantTitle: '助手',
    assistantDescription: '',
    assistantName: '助手名称',
    thread: '会话',
    keepsContext: '保留上下文',
    prompt: '提问',
    new: '新会话',
    send: '发送',
    promptHint: '',
    promptPlaceholder: '',
    noChat: '还没有本地对话。',
    messageRequired: '请输入内容',
    ready: '就绪',
    assistantReplied: '助手已回复',
    defaultAssistant: '默认助手',
    you: '你',
    assistantNeedsAttention: '助手需要处理',
    preferencesDescription: '',
    notLoaded: '未加载',
    edited: '已编辑',
    loaded: '已加载',
    saved: '已保存',
    saving: '保存中',
    savedRestart: '已保存，方便时重启生效',
    advancedPreferences: '高级偏好',
    reload: '重新加载',
    format: '格式化',
    save: '保存',
    formatted: '已格式化',
    recentMoments: '最近动态',
    recentDescription: '这次访问中发生的变化',
    events: (count) => `${count} 条动态`,
    nothingYet: '还没有动态。',
    atAGlance: '一览',
    summaryDescription: '当前工作台设置',
    savedAt: '保存位置',
    defaultAssistantLabel: '默认助手',
    listeningOn: '监听地址',
    assistants: '助手',
    noAssistants: '还没有配置助手。',
    workspaceRefreshed: '工作台已刷新',
    workspaceUnavailable: '工作台不可用',
    preferencesLoaded: '偏好已加载',
    preferencesSaved: '偏好已保存',
    preferencesUnavailable: '偏好不可用',
    preferencesSaveFailed: '偏好保存失败',
    settingsFile: '设置文件',
  },
};

function readPrefs() {
  try {
    const raw = window.localStorage.getItem(UI_PREFS_KEY);
    const prefs = raw ? JSON.parse(raw) : {};
    return {
      language: prefs.language === 'en' ? 'en' : 'zh',
      mode: prefs.mode === 'dark' ? 'dark' : 'light',
      theme: themes.some((theme) => theme.id === prefs.theme) ? prefs.theme : 'calm',
    };
  } catch {
    return { language: 'zh', mode: 'light', theme: 'calm' };
  }
}

function shortValue(value, fallback = 'None') {
  if (!value) return fallback;
  return value.length > 42 ? `${value.slice(0, 39)}...` : value;
}

function accountLabel(account) {
  return account.account_id || account.normalized_id || 'unknown';
}

function Notice({ value, type = '' }) {
  if (!value) return <div className="notice" aria-live="polite" />;
  const Icon = type === 'ok' ? Check : type === 'error' ? X : Clock;
  return (
    <div className={`notice ${type}`} aria-live="polite">
      <Icon size={16} aria-hidden="true" />
      <span>{value}</span>
    </div>
  );
}

function Field({ label, hint, htmlFor, children }) {
  return (
    <div className="field">
      <div className="field-row">
        <label htmlFor={htmlFor}>{label}</label>
        {hint ? <span className="hint">{hint}</span> : null}
      </div>
      {children}
    </div>
  );
}

function App() {
  const [uiPrefs, setUiPrefs] = useState(readPrefs);
  const t = copy[uiPrefs.language];
  const [view, setView] = useState('message');
  const [status, setStatus] = useState({ text: t.loading, type: '' });
  const [accounts, setAccounts] = useState([]);
  const [selected, setSelected] = useState('');
  const [activity, setActivity] = useState([]);
  const [notice, setNotice] = useState({ value: '', type: '' });
  const [chatNotice, setChatNotice] = useState({ value: '', type: '' });
  const [configNotice, setConfigNotice] = useState({ value: '', type: '' });
  const [isSending, setIsSending] = useState(false);
  const [isChatting, setIsChatting] = useState(false);
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [configLoaded, setConfigLoaded] = useState(false);
  const [configPath, setConfigPath] = useState('');
  const [configText, setConfigText] = useState('');
  const [agentOptions, setAgentOptions] = useState([]);
  const [chatLog, setChatLog] = useState([]);
  const [form, setForm] = useState({
    text: '',
    media_url: '',
  });
  const [chat, setChat] = useState({
    agent: '',
    conversation_id: 'web',
    message: '',
  });

  const currentAccount = useMemo(
    () => accounts.find((account) => account.account_id === selected),
    [accounts, selected],
  );
  const selectedLabel = currentAccount ? accountLabel(currentAccount) : t.noIdentity;

  function updatePrefs(nextPrefs) {
    setUiPrefs((current) => ({ ...current, ...nextPrefs }));
  }

  function addActivity(title, detail) {
    setActivity((items) => [
      {
        title,
        detail,
        time: new Date().toLocaleTimeString([], {
          hour: '2-digit',
          minute: '2-digit',
          second: '2-digit',
        }),
      },
      ...items,
    ].slice(0, 5));
  }

  async function loadStatus() {
    setNotice({ value: '', type: '' });
    setStatus({ text: t.loading, type: '' });
    const [statusResp, accountsResp] = await Promise.all([
      fetch('/api/status'),
      fetch('/api/accounts'),
    ]);
    if (!statusResp.ok) throw new Error(await statusResp.text());
    if (!accountsResp.ok) throw new Error(await accountsResp.text());
    const payload = await statusResp.json();
    const nextAccounts = await accountsResp.json();
    setAccounts(nextAccounts);
    setSelected((value) => {
      if (value && nextAccounts.some((account) => account.account_id === value)) return value;
      return nextAccounts[0]?.account_id || '';
    });
    setStatus({
      text: payload.account_count ? t.identitiesReady(payload.account_count) : t.readyLocal,
      type: 'ok',
    });
    addActivity(t.workspaceRefreshed, t.identitiesAvailable(payload.account_count));
  }

  function applyAgentOptions(cfg) {
    const agents = cfg.agents && typeof cfg.agents === 'object' ? cfg.agents : {};
    const agentNames = Object.keys(agents).sort();
    setAgentOptions(agentNames);
    setChat((value) => {
      if (value.agent || !cfg.default_agent || !agentNames.includes(cfg.default_agent)) {
        return value;
      }
      return { ...value, agent: cfg.default_agent };
    });
  }

  function applyConfigPayload(payload) {
    const cfg = payload.config || {};
    setConfigLoaded(true);
    setConfigPath(payload.path || '');
    setConfigText(JSON.stringify(cfg, null, 2));
    applyAgentOptions(cfg);
  }

  async function loadConfig() {
    setConfigNotice({ value: t.loading, type: 'pending' });
    const resp = await fetch('/api/config');
    if (!resp.ok) throw new Error(await resp.text());
    const payload = await resp.json();
    applyConfigPayload(payload);
    setConfigNotice({ value: t.loaded, type: 'ok' });
    addActivity(t.preferencesLoaded, payload.path || t.settingsFile);
  }

  async function loadAssistantOptions() {
    const resp = await fetch('/api/config');
    if (!resp.ok) return;
    const payload = await resp.json();
    applyAgentOptions(payload.config || {});
  }

  function parseConfig() {
    const trimmed = configText.trim();
    return trimmed ? JSON.parse(trimmed) : {};
  }

  async function saveConfig() {
    const cfg = parseConfig();
    setConfigText(JSON.stringify(cfg, null, 2));
    setConfigNotice({ value: t.saving, type: 'pending' });
    setIsSavingConfig(true);
    try {
      const resp = await fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cfg),
      });
      if (!resp.ok) throw new Error(await resp.text());
      const payload = await resp.json();
      applyConfigPayload(payload);
      const savedMessage = payload.reload_required ? t.savedRestart : t.saved;
      setConfigNotice({ value: savedMessage, type: 'ok' });
      addActivity(t.preferencesSaved, savedMessage);
    } finally {
      setIsSavingConfig(false);
    }
  }

  async function sendMessage(event) {
    event.preventDefault();
    const target = currentAccount?.user_id?.trim() || '';
    if (!target) {
      setNotice({ value: t.identityEmpty, type: 'error' });
      return;
    }
    setNotice({ value: t.sending, type: 'pending' });
    setIsSending(true);
    const payload = {
      account_id: selected,
      to: target,
      text: form.text,
      media_url: form.media_url.trim(),
    };
    try {
      const resp = await fetch('/api/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!resp.ok) throw new Error(await resp.text());
      setNotice({ value: t.sent, type: 'ok' });
      addActivity(t.messageSent, t.preparedFor(payload.to));
    } catch (err) {
      setNotice({ value: err.message || String(err), type: 'error' });
      addActivity(t.sendNeedsAttention, err.message || String(err));
    } finally {
      setIsSending(false);
    }
  }

  async function sendLocalChat(messageOverride = '', conversationOverride = '') {
    if (isChatting) {
      return;
    }
    const message = messageOverride || chat.message.trim();
    if (!message) {
      setChatNotice({ value: t.messageRequired, type: 'error' });
      return;
    }
    const conversationId = conversationOverride || chat.conversation_id.trim() || 'web';
    setChat((value) => ({
      ...value,
      conversation_id: conversationId,
      message: messageOverride ? value.message : '',
    }));
    setChatNotice({ value: t.sending, type: 'pending' });
    setIsChatting(true);
    if (!messageOverride) {
      setChatLog((items) => [...items, { role: 'user', label: t.you, text: message }]);
    }
    try {
      const resp = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          conversation_id: conversationId,
          agent: chat.agent.trim(),
          message,
        }),
      });
      if (!resp.ok) throw new Error(await resp.text());
      const payload = await resp.json();
      setChatLog((items) => [
        ...items,
        { role: 'agent', label: payload.agent || t.assistantTitle, text: payload.reply || '' },
      ]);
      setChatNotice({ value: t.ready, type: 'ok' });
      addActivity(t.assistantReplied, payload.agent || t.defaultAssistant);
    } catch (err) {
      setChatNotice({ value: err.message || String(err), type: 'error' });
      addActivity(t.assistantNeedsAttention, err.message || String(err));
    } finally {
      setIsChatting(false);
    }
  }

  useEffect(() => {
    loadStatus().catch((err) => {
      setStatus({ text: t.offline, type: 'error' });
      setNotice({ value: err.message || String(err), type: 'error' });
      addActivity(t.workspaceUnavailable, err.message || String(err));
    });
    loadAssistantOptions().catch(() => {});
  }, []);

  useEffect(() => {
    if (view === 'config' && !configLoaded) {
      loadConfig().catch((err) => {
        setConfigNotice({ value: err.message || String(err), type: 'error' });
        addActivity(t.preferencesUnavailable, err.message || String(err));
      });
    }
  }, [view, configLoaded]);

  useEffect(() => {
    document.documentElement.lang = uiPrefs.language === 'zh' ? 'zh-CN' : 'en';
    document.documentElement.dataset.mode = uiPrefs.mode;
    document.documentElement.dataset.theme = uiPrefs.theme;
    window.localStorage.setItem(UI_PREFS_KEY, JSON.stringify(uiPrefs));
  }, [uiPrefs]);

  useEffect(() => {
    setStatus((current) => {
      if (current.type === 'ok') {
        return {
          ...current,
          text: accounts.length ? t.identitiesReady(accounts.length) : t.readyLocal,
        };
      }
      if (current.type === 'error') {
        return { ...current, text: t.offline };
      }
      return { ...current, text: t.loading };
    });
  }, [uiPrefs.language, accounts.length]);

  const configSummary = useMemo(() => {
    try {
      const cfg = parseConfig();
      const agents = cfg.agents && typeof cfg.agents === 'object' ? cfg.agents : {};
      return {
        ok: true,
        config: cfg,
        agents: Object.keys(agents).sort().map((name) => ({ name, value: agents[name] || {} })),
      };
    } catch (err) {
      return { ok: false, error: err.message, config: {}, agents: [] };
    }
  }, [configText]);

  return (
    <>
      <header className="topbar">
        <div className="shell topbar-inner">
          <div className="brand">
            <div className="brand-mark" aria-hidden="true">
              <MessageCircle size={19} />
            </div>
            <div>
              <h1>Welinker</h1>
              {t.brandSubtitle ? <p className="brand-subtitle">{t.brandSubtitle}</p> : null}
            </div>
          </div>
          <div className="toolbar" aria-live="polite">
            <div className="status-pill">
              <span className={`dot ${status.type}`} />
              <span>{status.text}</span>
            </div>
            <div className="metric">
              <span>{t.identities}</span>
              <strong>{accounts.length}</strong>
            </div>
            <div className="quick-prefs" aria-label={`${t.language} / ${t.theme}`}>
              <div className="button-toggle" aria-label={t.language}>
                <button type="button" aria-pressed={uiPrefs.language === 'zh'} onClick={() => updatePrefs({ language: 'zh' })}>中</button>
                <button type="button" aria-pressed={uiPrefs.language === 'en'} onClick={() => updatePrefs({ language: 'en' })}>EN</button>
              </div>
              <div className="theme-buttons" aria-label={t.theme}>
                {themes.map((theme) => (
                  <button
                    key={theme.id}
                    className={`theme-dot theme-${theme.id}`}
                    type="button"
                    aria-label={theme.label[uiPrefs.language]}
                    aria-pressed={uiPrefs.theme === theme.id}
                    title={theme.label[uiPrefs.language]}
                    onClick={() => updatePrefs({ theme: theme.id })}
                  />
                ))}
              </div>
              <div className="button-toggle" aria-label={`${t.light} / ${t.dark}`}>
                <button type="button" aria-pressed={uiPrefs.mode === 'light'} onClick={() => updatePrefs({ mode: 'light' })}>
                  <Sun size={14} aria-hidden="true" />
                </button>
                <button type="button" aria-pressed={uiPrefs.mode === 'dark'} onClick={() => updatePrefs({ mode: 'dark' })}>
                  <Moon size={14} aria-hidden="true" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </header>

      <main className="shell">
        <nav className="view-tabs" aria-label="Welinker sections">
          <button className="tab-button" type="button" aria-selected={view === 'message'} onClick={() => setView('message')}>{t.compose}</button>
          <button className="tab-button" type="button" aria-selected={view === 'chat'} onClick={() => setView('chat')}>{t.assistant}</button>
          <button className="tab-button" type="button" aria-selected={view === 'config'} onClick={() => setView('config')}>{t.preferences}</button>
        </nav>

        {view === 'message' ? (
          <div className="view layout">
            <section className="panel" aria-labelledby="accountsTitle">
              <div className="panel-head">
                <div className="panel-title">
                  <Users size={18} aria-hidden="true" />
                  <div>
                    <h2 id="accountsTitle">{t.wechatIdentities}</h2>
                    {t.pickIdentity ? <p className="panel-description">{t.pickIdentity}</p> : null}
                  </div>
                </div>
                <button className="btn" type="button" title={t.refresh} onClick={() => loadStatus().catch((err) => setNotice({ value: err.message || String(err), type: 'error' }))}>
                  <RefreshCw size={16} aria-hidden="true" />
                  {t.refresh}
                </button>
              </div>
              <div className="panel-body account-list">
                {accounts.length ? accounts.map((account) => {
                  const id = account.account_id;
                  return (
                    <button key={id} type="button" className="account-card" aria-pressed={id === selected} onClick={() => setSelected(id)}>
                      <div className="account-meta">
                        <span>{t.accountId}: {account.account_id || t.unknown}</span>
                        <span>{t.wechatId}: {account.user_id || t.userUnknown}</span>
                      </div>
                    </button>
                  );
                }) : (
                  <div className="empty-state">
                    <AlertTriangle size={26} aria-hidden="true" />
                    <span>{t.identityEmpty}</span>
                  </div>
                )}
              </div>
            </section>

            <div>
              <section className="panel feature" aria-labelledby="sendTitle">
                <div className="panel-head">
                  <div className="panel-title">
                    <Send size={18} aria-hidden="true" />
                    <div>
                      <h2 id="sendTitle">{t.composeMessage}</h2>
                      {t.composeDescription ? <p className="panel-description">{t.composeDescription}</p> : null}
                    </div>
                  </div>
                  <span className="badge primary">{selectedLabel}</span>
                </div>
                <div className="panel-body">
                  <form onSubmit={sendMessage}>
                    <Field label={t.message} hint={`${form.text.length} ${t.characters}`} htmlFor="text">
                      <textarea className="control message-control" id="text" value={form.text} placeholder={t.messagePlaceholder} onChange={(event) => setForm({ ...form, text: event.target.value })} />
                    </Field>
                    <Field label={t.attachmentLink} hint={t.optional} htmlFor="mediaUrl">
                      <input className="control" id="mediaUrl" value={form.media_url} autoComplete="off" placeholder="https://example.com/image.png" onChange={(event) => setForm({ ...form, media_url: event.target.value })} />
                    </Field>
                    <div className="actions">
                      <Notice value={notice.value} type={notice.type} />
                      <div className="button-group">
                        <button className="btn" type="button" onClick={() => {
                          setForm({ text: '', media_url: '' });
                          setNotice({ value: '', type: '' });
                          addActivity(t.draftCleared, t.draftClearedDetail);
                        }}>
                          <Trash2 size={16} aria-hidden="true" />
                          {t.clear}
                        </button>
                        <button className="btn primary" type="submit" disabled={isSending}>
                          <Send size={16} aria-hidden="true" />
                          {t.sendMessage}
                        </button>
                      </div>
                    </div>
                  </form>
                </div>
              </section>

            </div>
          </div>
        ) : null}

        {view === 'chat' ? (
          <div className="view chat-layout">
            <section className="panel feature chat-panel" aria-labelledby="chatTitle">
              <div className="panel-head">
                <div className="panel-title">
                  <MessageCircle size={18} aria-hidden="true" />
                  <div>
                    <h2 id="chatTitle">{t.assistantTitle}</h2>
                    {t.assistantDescription ? <p className="panel-description">{t.assistantDescription}</p> : null}
                  </div>
                </div>
                <span className="badge neutral">{shortValue(chat.conversation_id.trim(), 'web')}</span>
              </div>
              <div className="panel-body">
                <form onSubmit={(event) => {
                  event.preventDefault();
                  sendLocalChat();
                }}>
                  <Field label={t.assistantName} hint={t.optional} htmlFor="chatAgent">
                    <select className="control" id="chatAgent" value={chat.agent} onChange={(event) => setChat({ ...chat, agent: event.target.value })}>
                      <option value="">{t.defaultAssistant}</option>
                      {agentOptions.map((name) => (
                        <option key={name} value={name}>{name}</option>
                      ))}
                    </select>
                  </Field>
                  <div className="chat-log" aria-live="polite">
                    {chatLog.length ? chatLog.map((entry, index) => (
                      <div className={`chat-bubble ${entry.role}`} key={`${entry.role}-${index}`}>
                        <div className="chat-role">{entry.label}</div>
                        <div className="chat-text">{entry.text}</div>
                      </div>
                    )) : <div className="empty-state">{t.noChat}</div>}
                  </div>
                  <Field label={t.prompt} hint={t.promptHint} htmlFor="chatMessage">
                    <textarea
                      className="control message-control"
                      id="chatMessage"
                      value={chat.message}
                      placeholder={t.promptPlaceholder}
                      onChange={(event) => setChat({ ...chat, message: event.target.value })}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                          event.preventDefault();
                          sendLocalChat();
                        }
                      }}
                    />
                  </Field>
                  <div className="actions">
                    <Notice value={chatNotice.value} type={chatNotice.type} />
                    <div className="button-group">
                      <button className="btn" type="button" onClick={() => {
                        const id = `web-${Date.now().toString(36)}`;
                        setChat((value) => ({ ...value, conversation_id: id }));
                        setChatLog([]);
                        sendLocalChat('/new', id);
                      }}>
                        <Plus size={16} aria-hidden="true" />
                        {t.new}
                      </button>
                      <button className="btn primary" type="submit" disabled={isChatting}>
                        <Send size={16} aria-hidden="true" />
                        {t.send}
                      </button>
                    </div>
                  </div>
                </form>
              </div>
            </section>
          </div>
        ) : null}

        {view === 'config' ? (
          <div className="view config-layout">
            <section className="panel feature" aria-labelledby="configTitle">
              <div className="panel-head">
                <div className="panel-title">
                  <Settings size={18} aria-hidden="true" />
                  <div>
                    <h2 id="configTitle">{t.preferences}</h2>
                    {t.preferencesDescription ? <p className="panel-description">{t.preferencesDescription}</p> : null}
                  </div>
                </div>
                <span className={configNotice.type === 'ok' ? 'badge primary' : 'badge neutral'}>{configNotice.value || (configLoaded ? t.edited : t.notLoaded)}</span>
              </div>
              <div className="panel-body">
                <form onSubmit={(event) => {
                  event.preventDefault();
                  saveConfig().catch((err) => {
                    setConfigNotice({ value: err.message || String(err), type: 'error' });
                    addActivity(t.preferencesSaveFailed, err.message || String(err));
                  });
                }}>
                  <Field label={t.advancedPreferences} hint={shortValue(configPath, emptyConfigPath)} htmlFor="configEditor">
                    <textarea className="control config-editor" id="configEditor" spellCheck="false" autoComplete="off" value={configText} onChange={(event) => {
                      setConfigText(event.target.value);
                      setConfigNotice({ value: '', type: '' });
                    }} />
                  </Field>
                  <div className="actions">
                    <Notice value={configNotice.value} type={configNotice.type} />
                    <div className="button-group">
                      <button className="btn" type="button" onClick={() => loadConfig().catch((err) => setConfigNotice({ value: err.message || String(err), type: 'error' }))}>
                        <RefreshCw size={16} aria-hidden="true" />
                        {t.reload}
                      </button>
                      <button className="btn" type="button" onClick={() => {
                        try {
                          const cfg = parseConfig();
                          setConfigText(JSON.stringify(cfg, null, 2));
                          setConfigNotice({ value: t.formatted, type: 'ok' });
                        } catch (err) {
                          setConfigNotice({ value: err.message || String(err), type: 'error' });
                        }
                      }}>
                        <ListChecks size={16} aria-hidden="true" />
                        {t.format}
                      </button>
                      <button className="btn primary" type="submit" disabled={isSavingConfig}>
                        <Save size={16} aria-hidden="true" />
                        {t.save}
                      </button>
                    </div>
                  </div>
                </form>
              </div>
            </section>
            <ConfigSummary summary={configSummary} path={configPath} t={t} />
          </div>
        ) : null}
      </main>
    </>
  );
}

function ActivityPanel({ items, t }) {
  return (
    <section className="panel activity" aria-labelledby="activityTitle">
      <div className="panel-head">
        <div className="panel-title">
          <Activity size={18} aria-hidden="true" />
          <div>
            <h2 id="activityTitle">{t.recentMoments}</h2>
            <p className="panel-description">{t.recentDescription}</p>
          </div>
        </div>
        <span className="badge neutral">{t.events(items.length)}</span>
      </div>
      <div className="panel-body activity-list">
        {items.length ? items.map((item, index) => (
          <div className="activity-item" key={`${item.time}-${index}`}>
            <div className="activity-main">
              <div className="activity-title">{item.title}</div>
              <div className="activity-detail">{item.detail}</div>
            </div>
            <div className="activity-time">{item.time}</div>
          </div>
        )) : <div className="empty-state">{t.nothingYet}</div>}
      </div>
    </section>
  );
}

function ConfigSummary({ summary, path, t }) {
  const config = summary.config || {};
  return (
    <section className="panel" aria-labelledby="configSummaryTitle">
      <div className="panel-head">
        <div className="panel-title">
          <FileJson size={18} aria-hidden="true" />
          <div>
            <h2 id="configSummaryTitle">{t.atAGlance}</h2>
            <p className="panel-description">{t.summaryDescription}</p>
          </div>
        </div>
      </div>
      <div className="panel-body config-summary">
        {!summary.ok ? <div className="notice error"><X size={16} aria-hidden="true" /><span>{summary.error}</span></div> : null}
        <div className="mini-stat"><span>{t.savedAt}</span><strong className="config-path">{path || emptyConfigPath}</strong></div>
        <div className="mini-stat"><span>{t.defaultAssistantLabel}</span><strong>{config.default_agent || '-'}</strong></div>
        <div className="mini-stat"><span>{t.listeningOn}</span><strong>{config.api_addr || '127.0.0.1:18011'}</strong></div>
        <div className="mini-stat"><span>{t.assistants}</span><strong>{summary.agents.length}</strong></div>
        <div className="agent-list">
          {summary.agents.length ? summary.agents.map(({ name, value }) => (
            <div className="agent-row" key={name}>
              <strong>{name}</strong>
              <span>{value.kind || value.type || 'unknown'} · {value.model || value.command || value.endpoint || 'default'}</span>
            </div>
          )) : <div className="empty-state">{t.noAssistants}</div>}
        </div>
      </div>
    </section>
  );
}

createRoot(document.getElementById('root')).render(<App />);
