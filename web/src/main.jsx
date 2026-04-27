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
  Plus,
  RefreshCw,
  Sparkles,
  Save,
  Send,
  Settings,
  Trash2,
  Users,
  X,
} from 'lucide-react';
import './styles.css';

const emptyConfigPath = '-';

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
  const [view, setView] = useState('message');
  const [status, setStatus] = useState({ text: 'Loading', type: '' });
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
  const [chatLog, setChatLog] = useState([]);
  const [form, setForm] = useState({
    to: '',
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
  const selectedLabel = currentAccount ? accountLabel(currentAccount) : 'No identity';

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
    setStatus({ text: 'Loading', type: '' });
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
      text: payload.account_count
        ? `${payload.account_count} WeChat identity${payload.account_count === 1 ? '' : 'ies'} ready`
        : 'Ready for local chat',
      type: 'ok',
    });
    addActivity('Workspace refreshed', `${payload.account_count} WeChat identity${payload.account_count === 1 ? '' : 'ies'} available`);
  }

  async function loadConfig() {
    setConfigNotice({ value: 'Loading', type: 'pending' });
    const resp = await fetch('/api/config');
    if (!resp.ok) throw new Error(await resp.text());
    const payload = await resp.json();
    setConfigLoaded(true);
    setConfigPath(payload.path || '');
    setConfigText(JSON.stringify(payload.config || {}, null, 2));
    setConfigNotice({ value: 'Loaded', type: 'ok' });
    addActivity('Preferences loaded', payload.path || 'settings file');
  }

  function parseConfig() {
    const trimmed = configText.trim();
    return trimmed ? JSON.parse(trimmed) : {};
  }

  async function saveConfig() {
    const cfg = parseConfig();
    setConfigText(JSON.stringify(cfg, null, 2));
    setConfigNotice({ value: 'Saving', type: 'pending' });
    setIsSavingConfig(true);
    try {
      const resp = await fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(cfg),
      });
      if (!resp.ok) throw new Error(await resp.text());
      const payload = await resp.json();
      setConfigLoaded(true);
      setConfigPath(payload.path || '');
      setConfigText(JSON.stringify(payload.config || {}, null, 2));
      const savedMessage = payload.reload_required ? 'Saved; restart when convenient' : 'Saved';
      setConfigNotice({ value: savedMessage, type: 'ok' });
      addActivity('Preferences saved', savedMessage);
    } finally {
      setIsSavingConfig(false);
    }
  }

  async function sendMessage(event) {
    event.preventDefault();
    setNotice({ value: 'Sending', type: 'pending' });
    setIsSending(true);
    const payload = {
      account_id: selected,
      to: form.to.trim(),
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
      setNotice({ value: 'Sent', type: 'ok' });
      addActivity('Message sent', `Prepared for ${payload.to}`);
    } catch (err) {
      setNotice({ value: err.message || String(err), type: 'error' });
      addActivity('Send needs attention', err.message || String(err));
    } finally {
      setIsSending(false);
    }
  }

  async function sendLocalChat(messageOverride = '') {
    const message = messageOverride || chat.message.trim();
    if (!message) {
      setChatNotice({ value: 'Message is required', type: 'error' });
      return;
    }
    const conversationId = chat.conversation_id.trim() || 'web';
    setChat((value) => ({ ...value, conversation_id: conversationId }));
    setChatNotice({ value: 'Sending', type: 'pending' });
    setIsChatting(true);
    if (!messageOverride) {
      setChatLog((items) => [...items, { role: 'user', label: 'You', text: message }]);
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
        { role: 'agent', label: payload.agent || 'Agent', text: payload.reply || '' },
      ]);
      setChatNotice({ value: 'Ready', type: 'ok' });
      addActivity('Assistant replied', payload.agent || 'default assistant');
      if (!messageOverride) {
        setChat((value) => ({ ...value, message: '' }));
      }
    } catch (err) {
      setChatNotice({ value: err.message || String(err), type: 'error' });
      addActivity('Assistant needs attention', err.message || String(err));
    } finally {
      setIsChatting(false);
    }
  }

  useEffect(() => {
    loadStatus().catch((err) => {
      setStatus({ text: 'offline', type: 'error' });
      setNotice({ value: err.message || String(err), type: 'error' });
      addActivity('Workspace unavailable', err.message || String(err));
    });
  }, []);

  useEffect(() => {
    if (view === 'config' && !configLoaded) {
      loadConfig().catch((err) => {
        setConfigNotice({ value: err.message || String(err), type: 'error' });
        addActivity('Preferences unavailable', err.message || String(err));
      });
    }
  }, [view, configLoaded]);

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
              <p className="brand-subtitle">a quiet desk for WeChat and your assistant</p>
            </div>
          </div>
          <div className="toolbar" aria-live="polite">
            <div className="status-pill">
              <span className={`dot ${status.type}`} />
              <span>{status.text}</span>
            </div>
            <div className="metric">
              <span>Identities</span>
              <strong>{accounts.length}</strong>
            </div>
            <div className="mood-pill">
              <Sparkles size={15} aria-hidden="true" />
              <span>Calm mode</span>
            </div>
          </div>
        </div>
      </header>

      <main className="shell">
        <nav className="view-tabs" aria-label="Welinker sections">
          <button className="tab-button" type="button" aria-selected={view === 'message'} onClick={() => setView('message')}>Compose</button>
          <button className="tab-button" type="button" aria-selected={view === 'chat'} onClick={() => setView('chat')}>Assistant</button>
          <button className="tab-button" type="button" aria-selected={view === 'config'} onClick={() => setView('config')}>Preferences</button>
        </nav>

        <section className="workspace-hero" aria-label="Workspace overview">
          <div>
            <p className="eyebrow">Today in Welinker</p>
            <h2>Send with a lighter touch.</h2>
            <p className="hero-copy">Choose an identity, write the message, and keep your assistant close when you need a second pass.</p>
          </div>
          <div className="hero-stats" aria-label="Workspace status">
            <div><span>Ready identities</span><strong>{accounts.length}</strong></div>
            <div><span>Current identity</span><strong>{shortValue(selectedLabel)}</strong></div>
          </div>
        </section>

        {view === 'message' ? (
          <div className="view layout">
            <section className="panel" aria-labelledby="accountsTitle">
              <div className="panel-head">
                <div className="panel-title">
                  <Users size={18} aria-hidden="true" />
                  <div>
                    <h2 id="accountsTitle">WeChat identities</h2>
                    <p className="panel-description">Pick where this message should come from</p>
                  </div>
                </div>
                <button className="btn" type="button" title="Refresh accounts" onClick={() => loadStatus().catch((err) => setNotice({ value: err.message || String(err), type: 'error' }))}>
                  <RefreshCw size={16} aria-hidden="true" />
                  Refresh
                </button>
              </div>
              <div className="panel-body account-list">
                {accounts.length ? accounts.map((account) => {
                  const id = account.account_id;
                  return (
                    <button key={id} type="button" className="account-card" aria-pressed={id === selected} onClick={() => setSelected(id)}>
                      <div className="account-top">
                        <div className="account-name">{accountLabel(account)}</div>
                        <span className={id === selected ? 'badge primary' : 'badge neutral'}>{id === selected ? 'In use' : 'Available'}</span>
                      </div>
                      <div className="account-meta">
                        <span>{account.user_id || 'user unknown'}</span>
                        <span>{account.base_url || 'default home'}</span>
                      </div>
                    </button>
                  );
                }) : (
                  <div className="empty-state">
                    <AlertTriangle size={26} aria-hidden="true" />
                    <span>No WeChat identity is connected yet.</span>
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
                      <h2 id="sendTitle">Compose message</h2>
                      <p className="panel-description">Write once, then send through the selected identity</p>
                    </div>
                  </div>
                  <span className="badge primary">{selectedLabel}</span>
                </div>
                <div className="panel-body">
                  <form onSubmit={sendMessage}>
                    <div className="form-grid">
                      <Field label="From" hint="Required" htmlFor="accountSelect">
                        <select className="control" id="accountSelect" value={selected} onChange={(event) => setSelected(event.target.value)}>
                          {accounts.length ? accounts.map((account) => (
                            <option key={account.account_id} value={account.account_id}>{accountLabel(account)}</option>
                          )) : <option value="">No identity</option>}
                        </select>
                      </Field>
                      <Field label="To" hint="WeChat contact id" htmlFor="to">
                        <input className="control" id="to" value={form.to} autoComplete="off" placeholder="user_id@im.wechat" required onChange={(event) => setForm({ ...form, to: event.target.value })} />
                      </Field>
                    </div>
                    <Field label="Message" hint={`${form.text.length} characters`} htmlFor="text">
                      <textarea className="control message-control" id="text" value={form.text} placeholder="Write something clear and kind." onChange={(event) => setForm({ ...form, text: event.target.value })} />
                    </Field>
                    <Field label="Attachment link" hint="Optional" htmlFor="mediaUrl">
                      <input className="control" id="mediaUrl" value={form.media_url} autoComplete="off" placeholder="https://example.com/image.png" onChange={(event) => setForm({ ...form, media_url: event.target.value })} />
                    </Field>
                    <div className="composer-meta" aria-label="Message details">
                      <div className="mini-stat"><span>From</span><strong>{shortValue(selectedLabel)}</strong></div>
                      <div className="mini-stat"><span>To</span><strong>{shortValue(form.to.trim(), 'Not set')}</strong></div>
                      <div className="mini-stat"><span>Attachment</span><strong>{shortValue(form.media_url.trim(), 'None')}</strong></div>
                    </div>
                    <div className="actions">
                      <Notice value={notice.value} type={notice.type} />
                      <div className="button-group">
                        <button className="btn" type="button" onClick={() => {
                          setForm({ to: '', text: '', media_url: '' });
                          setNotice({ value: '', type: '' });
                          addActivity('Draft cleared', 'The message area is fresh again');
                        }}>
                          <Trash2 size={16} aria-hidden="true" />
                          Clear
                        </button>
                        <button className="btn primary" type="submit" disabled={isSending}>
                          <Send size={16} aria-hidden="true" />
                          Send message
                        </button>
                      </div>
                    </div>
                  </form>
                </div>
              </section>

              <ActivityPanel items={activity} />
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
                    <h2 id="chatTitle">Assistant</h2>
                    <p className="panel-description">Draft, revise, and think beside the message</p>
                  </div>
                </div>
                <span className="badge neutral">{shortValue(chat.conversation_id.trim(), 'web')}</span>
              </div>
              <div className="panel-body">
                <form onSubmit={(event) => {
                  event.preventDefault();
                  sendLocalChat();
                }}>
                  <div className="form-grid">
                    <Field label="Assistant name" hint="Optional" htmlFor="chatAgent">
                      <input className="control" id="chatAgent" value={chat.agent} autoComplete="off" placeholder="default, codex, hermes" onChange={(event) => setChat({ ...chat, agent: event.target.value })} />
                    </Field>
                    <Field label="Thread" hint="Keeps context" htmlFor="chatConversation">
                      <input className="control" id="chatConversation" value={chat.conversation_id} autoComplete="off" onChange={(event) => setChat({ ...chat, conversation_id: event.target.value })} />
                    </Field>
                  </div>
                  <div className="chat-log" aria-live="polite">
                    {chatLog.length ? chatLog.map((entry, index) => (
                      <div className={`chat-bubble ${entry.role}`} key={`${entry.role}-${index}`}>
                        <div className="chat-role">{entry.label}</div>
                        <div className="chat-text">{entry.text}</div>
                      </div>
                    )) : <div className="empty-state">No local chat messages yet.</div>}
                  </div>
                  <Field label="Prompt" hint="Use New to reset" htmlFor="chatMessage">
                    <textarea className="control message-control" id="chatMessage" value={chat.message} placeholder="Ask for a rewrite, a reply idea, or a quick summary." onChange={(event) => setChat({ ...chat, message: event.target.value })} />
                  </Field>
                  <div className="actions">
                    <Notice value={chatNotice.value} type={chatNotice.type} />
                    <div className="button-group">
                      <button className="btn" type="button" onClick={() => {
                        const id = `web-${Date.now().toString(36)}`;
                        setChat((value) => ({ ...value, conversation_id: id }));
                        setChatLog([]);
                        sendLocalChat('/new');
                      }}>
                        <Plus size={16} aria-hidden="true" />
                        New
                      </button>
                      <button className="btn primary" type="submit" disabled={isChatting}>
                        <Send size={16} aria-hidden="true" />
                        Send
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
                    <h2 id="configTitle">Preferences</h2>
                    <p className="panel-description">Tune the workspace when you need deeper control</p>
                  </div>
                </div>
                <span className={configNotice.type === 'ok' ? 'badge primary' : 'badge neutral'}>{configNotice.value || (configLoaded ? 'Edited' : 'Not loaded')}</span>
              </div>
              <div className="panel-body">
                <form onSubmit={(event) => {
                  event.preventDefault();
                  saveConfig().catch((err) => {
                    setConfigNotice({ value: err.message || String(err), type: 'error' });
                    addActivity('Preferences save failed', err.message || String(err));
                  });
                }}>
                  <Field label="Advanced preferences" hint={shortValue(configPath, emptyConfigPath)} htmlFor="configEditor">
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
                        Reload
                      </button>
                      <button className="btn" type="button" onClick={() => {
                        try {
                          const cfg = parseConfig();
                          setConfigText(JSON.stringify(cfg, null, 2));
                          setConfigNotice({ value: 'Formatted', type: 'ok' });
                        } catch (err) {
                          setConfigNotice({ value: err.message || String(err), type: 'error' });
                        }
                      }}>
                        <ListChecks size={16} aria-hidden="true" />
                        Format
                      </button>
                      <button className="btn primary" type="submit" disabled={isSavingConfig}>
                        <Save size={16} aria-hidden="true" />
                        Save
                      </button>
                    </div>
                  </div>
                </form>
              </div>
            </section>
            <ConfigSummary summary={configSummary} path={configPath} />
          </div>
        ) : null}
      </main>
    </>
  );
}

function ActivityPanel({ items }) {
  return (
    <section className="panel activity" aria-labelledby="activityTitle">
      <div className="panel-head">
        <div className="panel-title">
          <Activity size={18} aria-hidden="true" />
          <div>
            <h2 id="activityTitle">Recent moments</h2>
            <p className="panel-description">What changed during this visit</p>
          </div>
        </div>
        <span className="badge neutral">{items.length} event{items.length === 1 ? '' : 's'}</span>
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
        )) : <div className="empty-state">Nothing has happened yet.</div>}
      </div>
    </section>
  );
}

function ConfigSummary({ summary, path }) {
  const config = summary.config || {};
  return (
    <section className="panel" aria-labelledby="configSummaryTitle">
      <div className="panel-head">
        <div className="panel-title">
          <FileJson size={18} aria-hidden="true" />
          <div>
            <h2 id="configSummaryTitle">At a glance</h2>
            <p className="panel-description">Current workspace settings</p>
          </div>
        </div>
      </div>
      <div className="panel-body config-summary">
        {!summary.ok ? <div className="notice error"><X size={16} aria-hidden="true" /><span>{summary.error}</span></div> : null}
        <div className="mini-stat"><span>Saved at</span><strong className="config-path">{path || emptyConfigPath}</strong></div>
        <div className="mini-stat"><span>Default assistant</span><strong>{config.default_agent || '-'}</strong></div>
        <div className="mini-stat"><span>Listening on</span><strong>{config.api_addr || '127.0.0.1:18011'}</strong></div>
        <div className="mini-stat"><span>Assistants</span><strong>{summary.agents.length}</strong></div>
        <div className="agent-list">
          {summary.agents.length ? summary.agents.map(({ name, value }) => (
            <div className="agent-row" key={name}>
              <strong>{name}</strong>
              <span>{value.kind || value.type || 'unknown'} · {value.model || value.command || value.endpoint || 'default'}</span>
            </div>
          )) : <div className="empty-state">No assistants are configured.</div>}
        </div>
      </div>
    </section>
  );
}

createRoot(document.getElementById('root')).render(<App />);
