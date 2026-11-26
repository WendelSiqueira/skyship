'use strict';

// Interface do skyship. Só traduz cliques em chamadas aos comandos do backend (Tauri) e mostra
// o andamento do deploy; nenhuma regra de deploy fica aqui.

const TAURI = window.__TAURI__;
const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);

const SCREENS = ['connect', 'project', 'deploy', 'live'];
const STAGES = ['build', 'service', 'upload', 'publish', 'wait_live'];
const LS_SIZES = ['nano', 'micro', 'small', 'medium', 'large'];
const EC2_SIZES = ['t3.micro', 't3.small', 't3.medium'];
const DEFAULT_PROFILE = 'skyship';
const DEFAULT_REGION = 'us-east-1';
// Troque a versão quando o texto dos termos mudar: o app pede o aceite de novo.
const TERMS_VERSION = '2026-11-26-5';
const TERMS_SECTIONS = 8;
const NAME_RE = /^[a-z0-9]([a-z0-9-]{0,38}[a-z0-9])?$/;
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;
const MAX_LOG_LINES = 800;
const TICK_ICON = '<svg class="tick" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M2.5 6.5l2.3 2.3L9.5 3.5"/></svg>';
const COPY_ICON ='<svg class="ic" width="14" height="14" viewBox="0 0 24 24" aria-hidden="true"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const EXT_ICON = '<svg class="ic" width="14" height="14" viewBox="0 0 24 24" aria-hidden="true"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><path d="M15 3h6v6"/><path d="M10 14L21 3"/></svg>';

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

const store = {
  get: (k) => { try { return localStorage.getItem('skyship.' + k); } catch { return null; } },
  set: (k, v) => { try { localStorage.setItem('skyship.' + k, v); } catch { /* sem armazenamento */ } },
  del: (k) => { try { localStorage.removeItem('skyship.' + k); } catch { /* idem */ } },
};

function initialLang() {
  const saved = store.get('lang');
  if (saved && I18N[saved]) return saved;
  const nav = (navigator.language || 'en').slice(0, 2);
  return I18N[nav] ? nav : 'en';
}
function initialTheme() {
  const saved = store.get('theme');
  if (saved === 'light' || saved === 'dark') return saved;
  return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

const state = {
  lang: initialLang(),
  theme: initialTheme(),
  screen: 'connect',
  profile: null,
  account: null,
  regions: [],
  dir: null,
  info: null,
  draft: { name: '', region: DEFAULT_REGION, port: '', target: 'lightsail', size: 'nano' },
  stageState: {},
  stageStart: {},
  stageSecs: {},
  logLines: [],
  deploying: false,
  deployDone: false,
  deployFailed: false,
  deployCancelled: false,
  url: null,
  deployedAt: null,
  apps: null, // { region, res: { apps, warnings } } da última listagem
  termsOk: false, // termos da versão atual aceitos
  termsReadOnly: false, // true quando os termos foram abertos pelo menu, depois do aceite
  prevScreen: 'connect',
  readProfiles: true, // permissão para listar os nomes dos perfis AWS (ligada por padrão)
  profiles: null, // nomes dos perfis; null = ainda não carregou
  profileAccounts: {}, // perfil -> conta, para os que já foram conferidos
  pmenuError: '', // erro ao trocar de perfil, mostrado dentro do menu
  checkingProfile: null, // perfil que está sendo conferido agora
  checking: false, // verificando conta e apps na abertura
  appsRegion: null, // região da lista de apps
  warnOpen: false, // detalhes completos do aviso da lista de apps abertos
  deleteUnmanaged: false, // pode apagar serviços sem a tag do skyship (desligado por padrão)
  scanImages: false, // verificar a imagem com o Trivy antes de publicar
  trivy: null, // o Docker responde para rodar o Trivy? (null = ainda não checado)
  existing: null, // apps da conta na região do formulário: { region, apps }
  dockerList: null, // permissão de listar o Docker: true | false | null (nunca perguntado)
  source: 'folder', // origem do app: 'folder' | 'docker' | 'archive'
  infoSource: null, // origem da análise guardada em state.info
  image: null, // referência da imagem escolhida (origem 'docker')
  docker: null, // { images, containers } da última leitura
  dockerLoading: false,
  dockerError: '',
  dockerAll: false, // mostrar também imagens sem porta e fora de uso
  archiveName: '', // nome do arquivo compactado escolhido
  settingsReturn: 'connect', // tela para onde "Voltar" leva nas configurações
};

/* ---------- textos ---------- */

function t(key, vars) {
  let s = (I18N[state.lang] && I18N[state.lang][key]) ?? I18N.pt[key] ?? key;
  if (vars) for (const k of Object.keys(vars)) s = s.split('{' + k + '}').join(String(vars[k]));
  return s;
}

function applyI18n() {
  document.documentElement.lang = { pt: 'pt-BR', en: 'en', es: 'es' }[state.lang] || 'en';
  $$('[data-i18n]').forEach((el) => { el.textContent = t(el.dataset.i18n); });
  $$('[data-i18n-aria]').forEach((el) => el.setAttribute('aria-label', t(el.dataset.i18nAria)));
  $$('[data-i18n-title]').forEach((el) => { el.title = t(el.dataset.i18nTitle); });
  renderAll();
}

function setLang(lang) {
  if (!I18N[lang]) return;
  state.lang = lang;
  store.set('lang', lang);
  applyI18n();
  syncMenu();
}

function setTheme(theme) {
  state.theme = theme;
  store.set('theme', theme);
  document.documentElement.dataset.theme = theme;
  syncMenu();
}

/* ---------- utilidades ---------- */

function showError(el, msg) {
  el.textContent = msg || '';
  el.hidden = !msg;
}

async function copyText(text, btn) {
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try { ok = document.execCommand('copy'); } catch { ok = false; }
    ta.remove();
  }
  if (btn) showTip(btn, ok ? t('copied') : t('copyFailed'));
  return ok;
}

let tipTimer = 0;
/** Balãozinho curto acima de `anchor` (ex.: "Copiado"), some sozinho. */
function showTip(anchor, text) {
  const tip = $('#tip');
  tip.textContent = text;
  tip.hidden = false;
  const r = anchor.getBoundingClientRect();
  const half = tip.offsetWidth / 2;
  const x = Math.min(Math.max(r.left + r.width / 2, half + 8), window.innerWidth - half - 8);
  tip.style.left = x + 'px';
  tip.style.top = Math.max(r.top - 8, tip.offsetHeight + 8) + 'px';
  clearTimeout(tipTimer);
  tipTimer = setTimeout(() => { tip.hidden = true; }, 1500);
}

/** Abre o endereço no navegador do sistema. Se não der, copia o endereço (e o balão "Copiado" avisa). */
function openUrl(url, btn) {
  if (!/^https?:\/\//i.test(url)) return;
  const fallback = () => { if (btn) copyText(url, btn); };
  const opener = TAURI.opener;
  if (opener && opener.openUrl) {
    Promise.resolve(opener.openUrl(url)).catch(fallback);
  } else {
    fallback();
  }
}

function shortAccount(acc) {
  return acc && acc.length > 8 ? acc.slice(0, 4) + '…' + acc.slice(-4) : acc || '';
}

function regionLabel(code) {
  const r = state.regions.find((x) => x.code === code);
  return r ? `${r.code} (${r.name})` : code;
}

function confirmModal({ title, text, yes, no, danger = true }) {
  return new Promise((resolve) => {
    const modal = $('#modal');
    $('#modal-title').textContent = title;
    $('#modal-text').textContent = text;
    $('#modal-yes').className = 'btn ' + (danger ? 'danger-btn' : 'primary');
    $('#modal-yes').textContent = yes;
    $('#modal-no').textContent = no;
    const done = (v) => {
      modal.hidden = true;
      $('#modal-yes').onclick = $('#modal-no').onclick = null;
      document.removeEventListener('keydown', onKey);
      resolve(v);
    };
    const onKey = (e) => { if (e.key === 'Escape') done(false); };
    document.addEventListener('keydown', onKey);
    $('#modal-yes').onclick = () => done(true);
    $('#modal-no').onclick = () => done(false);
    modal.hidden = false;
    $('#modal-no').focus();
  });
}

/* ---------- barra lateral ---------- */

// Itens do menu lateral, na ordem em que aparecem. O deploy não é um item: abre sozinho ao publicar
// e conta como parte de "Novo deploy".
const NAV = [
  { key: 'live', icon: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>' },
  { key: 'project', icon: '<path d="M12 5v14M5 12h14"/>' },
  { key: 'connect', icon: '<circle cx="8" cy="15" r="4"/><path d="M10.8 12.2L20 3M17 6l3 3M14 9l2 2"/>' },
];
const NAV_OF = { connect: 'connect', project: 'project', deploy: 'project', live: 'live' };

function renderSteps() {
  const labels = t('steps').split('|'); // na ordem de NAV
  const cur = NAV_OF[state.screen] ?? null; // termos e configurações não são itens do menu
  $('#steps').innerHTML = NAV.map((n, i) => {
    const needsAccount = !state.termsOk || (n.key !== 'connect' && !state.account && !state.checking);
    const busy = state.deploying && n.key !== cur; // durante o deploy só dá para cancelar
    const lockAttrs = needsAccount ? ` disabled title="${t('step.needAccount')}"` : busy ? ` disabled title="${t('step.busy')}"` : '';
    const icon = `<svg class="nav-ic" width="18" height="18" viewBox="0 0 24 24" aria-hidden="true">${n.icon}</svg>`;
    return `<button type="button" class="step${n.key === cur ? ' current' : ''}" data-nav="${n.key}"${n.key === cur ? ' aria-current="page"' : ''}${lockAttrs}>${icon}<span>${labels[i]}</span></button>`;
  }).join('');
}

function renderFoot() {
  const text = state.checking
    ? t('f.checking')
    : state.account
      ? t('f.account', { acc: shortAccount(state.account), profile: state.profile || DEFAULT_PROFILE })
      : t('f.none');
  $('#foot-text').textContent = text + ' ▾'; // espaço sem quebra: a seta não fica sozinha na linha
  if (!$('#pmenu').hidden) renderProfileMenu();
}

function syncMenu() {
  $$('#menu [data-lang]').forEach((b) => b.setAttribute('aria-checked', String(b.dataset.lang === state.lang)));
  $$('#menu [data-theme]').forEach((b) => b.setAttribute('aria-checked', String(b.dataset.theme === state.theme)));
}

function toggleMenu(open) {
  const menu = $('#menu');
  const willOpen = open ?? menu.hidden;
  menu.hidden = !willOpen;
  $('#gear').setAttribute('aria-expanded', String(willOpen));
  if (willOpen) togglePMenu(false);
}

function togglePMenu(open) {
  const menu = $('#pmenu');
  const willOpen = open ?? menu.hidden;
  menu.hidden = !willOpen;
  $('#foot-text').setAttribute('aria-expanded', String(willOpen));
  if (willOpen) {
    toggleMenu(false);
    state.pmenuError = '';
    renderProfileMenu();
    if (state.profiles === null && state.readProfiles) loadProfiles();
  }
}

/* ---------- navegação ---------- */

function showScreen(name) {
  state.screen = name;
  $$('section[data-screen]').forEach((s) => { s.hidden = s.dataset.screen !== name; });
  $('.main').scrollTop = 0;
  renderSteps();
  renderAll();
}

function renderAll() {
  renderSteps();
  renderFoot();
  syncMenu();
  if (state.screen === 'settings') renderSettings();
  if (state.screen === 'terms') renderTerms();
  if (state.screen === 'connect') renderConnect();
  if (state.screen === 'project') renderProject();
  if (state.screen === 'deploy') renderDeploy();
  if (state.screen === 'live') renderLive();
}

/** Vai para um passo da barra lateral. Qualquer passo vale depois de conectar, mesmo sem dados. */
function goStep(key) {
  if (!state.termsOk || (key !== 'connect' && !state.account) || state.deploying) return;
  if (key === 'connect') showScreen('connect');
  else if (key === 'project') enterProject();
  else if (key === 'live') { showScreen('live'); loadApps(); }
}

/* ---------- 1. conectar ---------- */

function renderConnect() {
  const note = $('#connected-note');
  if (state.account) {
    note.textContent = t('c.connected', { acc: shortAccount(state.account), profile: state.profile || DEFAULT_PROFILE });
    note.hidden = false;
  } else {
    note.hidden = true;
  }
}

/* ---------- perfis AWS deste computador ---------- */

async function loadProfiles() {
  state.profiles = null;
  if (state.readProfiles) {
    try { state.profiles = await invoke('profiles'); } catch { state.profiles = []; }
  }
  if (!$('#pmenu').hidden) renderProfileMenu();
}

/** Menu de perfis (rodapé da barra lateral): lista os perfis e deixa trocar entre eles. */
function menuButton(action, label) {
  const b = el('button', 'menu-item');
  b.type = 'button';
  b.setAttribute('role', 'menuitem');
  b.dataset.action = action;
  b.append(el('span', 'menu-label', label));
  return b;
}

function renderProfileMenu() {
  const menu = $('#pmenu');
  menu.textContent = '';
  menu.append(el('div', 'menu-title', t('pf.menuTitle')));
  if (!state.readProfiles) {
    menu.append(el('div', 'menu-note', t('pf.off')));
  } else if (state.profiles === null) {
    menu.append(el('div', 'menu-note', t('pf.loading')));
  } else if (!state.profiles.length) {
    menu.append(el('div', 'menu-note', t('pf.none')));
  } else {
    state.profiles.forEach((name) => {
      const active = Boolean(state.account) && name === state.profile;
      const b = el('button', 'menu-item');
      b.type = 'button';
      b.setAttribute('role', 'menuitemradio');
      b.setAttribute('aria-checked', String(active));
      b.dataset.profile = name;
      const label = el('span', 'menu-label pcol');
      label.append(el('span', 'pname', name));
      const acc = active ? state.account : state.profileAccounts[name];
      if (state.checkingProfile === name) label.append(el('small', '', t('pf.checking')));
      else if (acc) label.append(el('small', '', t('pf.account', { acc: shortAccount(acc) })));
      b.append(label);
      b.insertAdjacentHTML('beforeend', TICK_ICON);
      menu.append(b);
    });
  }
  if (state.pmenuError) menu.append(el('div', 'menu-error', state.pmenuError));
  menu.append(el('div', 'menu-sep'));
  menu.append(menuButton('keys', t('pf.keys')));
  if (!state.readProfiles) menu.append(menuButton('terms', t('t.menu')));
}

async function switchProfile(profile) {
  if (state.checkingProfile || state.deploying) return;
  state.pmenuError = '';
  state.checkingProfile = profile;
  renderProfileMenu();
  try {
    const account = await invoke('check_profile', { profile, region: DEFAULT_REGION });
    state.checkingProfile = null;
    togglePMenu(false);
    await useAccount(profile, account);
  } catch (e) {
    state.checkingProfile = null;
    state.pmenuError = String(e);
    renderProfileMenu();
  }
}

/* ---------- termos de uso ---------- */

function renderTerms() {
  const box = $('#terms-text');
  box.textContent = '';
  for (let i = 1; i <= TERMS_SECTIONS; i++) box.append(el('h3', '', t(`t.s${i}.h`)), el('p', '', t(`t.s${i}.b`)));
  $('#terms-go').textContent = t(state.termsReadOnly ? 't.save' : 't.go');
  $('#terms-go').disabled = !$('#terms-accept').checked;
  $('#terms-back').hidden = !state.termsReadOnly;
}

function openTerms(readOnly) {
  if (state.screen === 'terms') return;
  state.termsReadOnly = readOnly;
  state.prevScreen = state.screen;
  $('#terms-accept').checked = state.termsOk;
  $('#terms-profiles').checked = state.readProfiles;
  showScreen('terms');
  $('#terms-text').scrollTop = 0;
}

async function acceptTerms() {
  state.termsOk = true;
  state.readProfiles = $('#terms-profiles').checked;
  store.set('terms', TERMS_VERSION);
  // a permissão fica no backend (arquivo de configuração), que é quem a confere
  try { await invoke('update_settings', { readProfiles: state.readProfiles }); } catch { /* segue */ }
  if (state.termsReadOnly) {
    state.termsReadOnly = false;
    showScreen(state.prevScreen || 'connect');
    if (state.screen === 'connect') loadProfiles();
    return;
  }
  await startAfterTerms();
}

async function startAfterTerms() {
  // Abre em Apps (com "carregando") enquanto o perfil salvo é conferido; só cai em Credenciais
  // se nenhum perfil funcionar.
  state.checking = true;
  showScreen('live');
  await loadProfiles();
  // Perfil escolhido da última vez e, se existir, o `skyship` (criado pelo próprio app). Qualquer
  // outro perfil só é usado quando a pessoa escolhe, no menu do rodapé.
  const candidates = [store.get('profile'),
    state.readProfiles && (state.profiles || []).includes(DEFAULT_PROFILE) ? DEFAULT_PROFILE : null].filter(Boolean);
  for (const profile of [...new Set(candidates)]) {
    state.checking = true;
    renderFoot();
    try {
      const account = await invoke('check_profile', { profile, region: DEFAULT_REGION });
      state.checking = false;
      await useAccount(profile, account); // abre os apps
      return;
    } catch {
      if (profile === store.get('profile')) store.del('profile');
    }
  }
  state.checking = false;
  showScreen('connect');
}

async function loadRegions() {
  try { state.regions = await invoke('regions'); } catch { state.regions = []; }
}

async function useAccount(profile, account) {
  state.profile = profile;
  state.account = account;
  state.profileAccounts[profile] = account;
  state.apps = null; // a lista de apps é da conta anterior
  state.url = null;
  store.set('profile', profile);
  renderFoot();
  await routeAfterAccount();
}

/** Depois de conectar (ou ao abrir com uma conta salva) o app sempre começa em "Apps". */
async function routeAfterAccount() {
  state.appsRegion = store.get('region') || DEFAULT_REGION;
  showScreen('live');
  await loadApps();
}

async function connectWithKeys() {
  const err = $('#connect-error');
  const keyId = $('#key-id').value.trim();
  const secret = $('#secret').value.trim();
  showError(err, '');
  if (!keyId || !secret) { showError(err, t('c.needKeys')); return; }
  const btn = $('#connect-go');
  btn.disabled = true;
  btn.textContent = t('c.validating');
  try {
    const res = await invoke('save_credentials', { profile: DEFAULT_PROFILE, keyId, secret, region: DEFAULT_REGION });
    $('#secret').value = '';
    useAccount(DEFAULT_PROFILE, res.account);
  } catch (e) {
    showError(err, String(e));
  } finally {
    btn.disabled = false;
    btn.textContent = t('c.go');
  }
}

/* ---------- 2. projeto ---------- */

async function enterProject() {
  state.existing = null; // a lista pode ter mudado desde a última vez
  showScreen('project');
  loadExisting();
  try {
    const ok = await invoke('docker_ok');
    const w = $('#docker-warn');
    w.textContent = ok ? '' : t('p.dockerDown');
    w.hidden = ok;
  } catch { /* ignora */ }
}

/** Apps da conta na região escolhida, para saber se publicar vai atualizar um que já existe. */
let existingSeq = 0;
const EXISTING_TIMEOUT_MS = 15000;

async function loadExisting() {
  if (!state.account) return;
  const region = state.draft.region;
  if (state.existing && state.existing.region === region) return;
  if (state.existingLoading === region) return; // já buscando esta região
  state.existingLoading = region;
  const seq = ++existingSeq;
  // Se a consulta demorar demais, o botão não fica bloqueado para sempre: trata como app novo
  // (e, se a resposta chegar depois, ela substitui isto e a tela se corrige).
  const giveUp = setTimeout(() => {
    if (seq !== existingSeq || (state.existing && state.existing.region === region)) return;
    state.existing = { region, apps: [] };
    if (state.screen === 'project') renderProject();
  }, EXISTING_TIMEOUT_MS);
  try {
    const res = await invoke('list_apps', { profile: state.profile, region });
    if (seq !== existingSeq) { state.existingLoading = null; return; }
    state.existing = { region, apps: res.apps || [] };
  } catch {
    if (seq !== existingSeq) { state.existingLoading = null; return; }
    state.existing = { region, apps: [] }; // sem a lista, trata como app novo
  } finally {
    clearTimeout(giveUp);
  }
  state.existingLoading = null;
  if (state.screen === 'project') renderProject();
}

/** Ainda estamos descobrindo se já existe um app com esse nome na região escolhida? */
function checkingExisting() {
  return Boolean(state.account) && (!state.existing || state.existing.region !== state.draft.region);
}

function findExisting() {
  const d = state.draft;
  const e = state.existing;
  if (!e || e.region !== d.region || !d.name) return null;
  return e.apps.find((a) => a.name === d.name && a.target === d.target) || null;
}

/** Botão e aviso: "Atualizar X" quando já existe um app com esse nome, destino e região. */
function renderUpdateNote() {
  const d = state.draft;
  const ex = findExisting();
  const note = $('#update-note');
  const checking = checkingExisting();
  if (!state.deploying) {
    // enquanto a conta é consultada o botão fica bloqueado: publicar antes disso passaria por cima de um app que já existe
    $('#publish').textContent = checking ? t('p.checking') : ex ? t('p.update', { name: d.name }) : t('p.publish');
    $('#publish').disabled = checking || !canPublish(currentInfo());
  }
  if (!ex) { note.hidden = true; return; }
  const lines = [t('p.updateNote', { name: d.name, dest: d.target === 'ec2' ? 'EC2' : 'Lightsail', region: regionLabel(d.region), state: ex.state })];
  if (d.target === 'ec2') lines.push(t('p.updateEc2'));
  if (ex.size && d.size && ex.size !== d.size) lines.push(t('p.updateSize', { cur: ex.size, want: d.size }));
  note.textContent = lines.join('\n');
  note.hidden = false;
}

function sizesFor(target) { return target === 'ec2' ? EC2_SIZES : LS_SIZES; }

function regionsFor(target) {
  const all = state.regions;
  if (target !== 'lightsail') return all;
  const ls = all.filter((r) => r.lightsail);
  return ls.length ? ls : all;
}

function fillOptions() {
  const d = state.draft;
  const regs = regionsFor(d.target);
  if (!regs.some((r) => r.code === d.region)) d.region = (regs.find((r) => r.code === DEFAULT_REGION) || regs[0] || { code: DEFAULT_REGION }).code;
  $('#region').innerHTML = regs.map((r) => `<option value="${r.code}">${r.code} (${r.name})</option>`).join('');
  $('#region').value = d.region;

  const sizes = sizesFor(d.target);
  if (!sizes.includes(d.size)) d.size = sizes[0];
  $('#size').innerHTML = sizes.map((s) => `<option value="${s}">${sizeLabel(d.target, s)}</option>`).join('');
  $('#size').value = d.size;
}

function sizeLabel(target, size) {
  return size;
}

function renderProject() {
  const info = currentInfo();
  const d = state.draft;
  renderSource(info);

  const det = $('#detect');
  det.className = 'banner';
  if (state.source === 'docker') {
    renderDockerDetect(det);
  } else if (!info) {
    det.hidden = true;
  } else if (info.config_error) {
    det.className = 'banner warn';
    det.textContent = t('p.configErr', { err: info.config_error });
    det.hidden = false;
  } else if (!info.has_dockerfile) {
    det.className = 'banner warn';
    det.textContent = t('p.noDockerfile');
    det.hidden = false;
  } else {
    det.textContent = info.port ? t('p.detected', { port: info.port }) : t('p.noPort');
    det.hidden = false;
  }

  $$('.choice[data-target]').forEach((c) => {
    const on = c.dataset.target === d.target;
    c.classList.toggle('selected', on);
    c.setAttribute('aria-checked', String(on));
  });
  fillOptions();
  $('#dk-scan').checked = state.scanImages;
  const sh = $('#dk-scan-hint');
  sh.hidden = !state.scanImages || state.trivy === null;
  sh.textContent = state.trivy === false ? t('p.scanMissing') : t('p.scanSlow');
  if (state.scanImages && state.trivy === null && !state.trivyChecking) {
    state.trivyChecking = true;
    checkTrivy().then(() => { state.trivyChecking = false; if (state.screen === 'project') renderProject(); });
  }
  $('#publish').disabled = !canPublish(info) || state.deploying;
  renderUpdateNote();
  loadExisting(); // só busca de novo se a região mudou
}

/** Análise da pasta/arquivo só vale se veio da origem que está selecionada. */
function currentInfo() {
  return state.source !== 'docker' && state.infoSource === state.source ? state.info : null;
}

function canPublish(info) {
  return state.source === 'docker' ? Boolean(state.image) : Boolean(info && info.has_dockerfile);
}

function renderSource(info) {
  const s = state.source;
  $$('.choice[data-source]').forEach((c) => {
    const on = c.dataset.source === s;
    c.classList.toggle('selected', on);
    c.setAttribute('aria-checked', String(on));
  });
  $('#src-folder').hidden = s !== 'folder';
  $('#src-archive').hidden = s !== 'archive';
  $('#archive-hint').hidden = s !== 'archive';
  $('#dk-panel').hidden = s !== 'docker';
  $('#dir-box').textContent = info && s === 'folder' ? info.dir : t('p.noFolder');
  $('#archive-box').textContent = state.archiveName || t('p.noArchive');
  if (s === 'docker') renderDocker();
}

async function setSource(src) {
  if (state.source === src) return;
  readProjectForm();
  state.source = src;
  // nome e porta sugeridos pela origem anterior não valem para a nova; o que a pessoa digitou fica
  const d = state.draft;
  if (!d.typedName) { d.name = ''; $('#app-name').value = ''; }
  if (!d.typedPort) { d.port = ''; $('#app-port').value = ''; }
  state.image = null;
  showError($('#project-error'), '');
  $('#live-banner').hidden = true;
  renderProject();
  if (src === 'docker') await enterDockerSource();
}

async function pickFolder() {
  let dir = null;
  try {
    dir = await TAURI.dialog.open({ directory: true, multiple: false, title: t('p.pick') });
  } catch { return; }
  if (!dir) return;
  await loadProject(Array.isArray(dir) ? dir[0] : dir);
}

async function loadProject(dir) {
  const err = $('#project-error');
  showError(err, '');
  $('#live-banner').hidden = true;
  let info;
  try {
    info = await invoke('inspect_project', { dir });
  } catch (e) {
    state.info = null;
    renderProject();
    showError(err, String(e));
    return;
  }
  await applyProject(info);
}

/** Aplica a análise de uma pasta (escolhida ou extraída de um arquivo) ao formulário. */
async function applyProject(info) {
  showError($('#project-error'), '');
  state.info = info;
  state.infoSource = state.source;
  state.dir = info.dir;

  const cfg = info.config;
  const d = state.draft;
  d.name = cfg ? cfg.name : info.suggested_name;
  d.port = cfg ? cfg.port : (info.port ?? '');
  d.target = cfg ? cfg.target : 'lightsail';
  d.region = cfg ? cfg.region : (store.get('region') || DEFAULT_REGION);
  d.size = cfg ? (cfg.target === 'ec2' ? cfg.ec2.instance_type : cfg.lightsail.power) : sizesFor(d.target)[0];
  d.typedName = d.typedPort = false;
  $('#app-name').value = d.name;
  $('#app-port').value = d.port;
  renderProject();

  if (cfg) {
    try {
      const entries = await invoke('status', { dir: info.dir, profile: state.profile });
      const found = entries.find((e) => e.url);
      if (found) {
        $('#live-banner-text').textContent = t('p.alreadyLive', { url: found.url });
        $('#live-banner-go').hidden = false;
        $('#live-banner').hidden = false;
        $('#live-banner-go').onclick = () => { state.url = found.url; state.deployedAt = null; showScreen('live'); loadApps(); };
      }
    } catch { /* sem status: segue normal */ }
  }
}

function readProjectForm() {
  const d = state.draft;
  d.name = $('#app-name').value.trim();
  d.port = $('#app-port').value.trim();
  d.region = $('#region').value;
  d.size = $('#size').value;
}

async function publish() {
  if (checkingExisting()) return;
  const err = $('#project-error');
  showError(err, '');
  readProjectForm();
  const d = state.draft;
  if (!NAME_RE.test(d.name)) { showError(err, t('p.badName')); return; }
  const port = Number(d.port);
  if (!Number.isInteger(port) || port < 1 || port > 65535) { showError(err, t('p.noPortField')); return; }
  const fromImage = state.source === 'docker';
  if (fromImage && !state.image) { showError(err, t('p.noImage')); return; }
  const info = currentInfo();
  const cfg = info && info.config;
  const input = {
    name: d.name,
    region: d.region,
    port,
    target: d.target,
    instanceType: d.target === 'ec2' ? d.size : (cfg ? cfg.ec2.instance_type : 't3.micro'),
    power: d.target === 'lightsail' ? d.size : (cfg ? cfg.lightsail.power : 'nano'),
    image: fromImage ? state.image : null,
  };
  try {
    // no modo imagem não há pasta: o backend guarda a config numa pasta própria e devolve o caminho
    state.dir = await invoke('save_config', { dir: fromImage ? null : state.dir, input });
  } catch (e) {
    showError(err, String(e));
    return;
  }
  store.set('region', d.region);
  if (!(await scanBeforePublish())) return;
  await startDeploy();
}

/* ---------- origem: imagens do Docker ---------- */

function slug(ref) {
  const base = ref.split('@')[0].split(':')[0].split('/').pop();
  const s = base.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40).replace(/-+$/, '');
  return s || 'app';
}

function fmtSize(bytes) {
  if (bytes >= 1e9) return (bytes / 1e9).toFixed(2).replace(/\.?0+$/, '') + ' GB';
  return Math.max(1, Math.round(bytes / 1e6)) + ' MB';
}

function ago(iso) {
  const ms = Date.now() - Date.parse(iso);
  if (!Number.isFinite(ms)) return '';
  const min = Math.floor(ms / 60000);
  if (min < 1) return t('ago.now');
  if (min < 60) return t('ago.min', { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t('ago.hour', { n: h });
  const d = Math.floor(h / 24);
  if (d < 60) return t('ago.day', { n: d });
  return t('ago.month', { n: Math.floor(d / 30) });
}

function findImage(ref) {
  return state.docker ? state.docker.images.find((i) => i.reference === ref) : null;
}

function projectDirFor(img) {
  const c = state.docker && state.docker.containers.find((x) => x.image_id === img.id && x.project_dir);
  return c ? c.project_dir : null;
}

async function setDockerPermission(allowed) {
  const s = await invoke('update_settings', { dockerList: allowed });
  state.dockerList = s.dockerList;
  if (!allowed) { state.docker = null; state.image = null; }
}

/** Pede a permissão para ler o Docker. Devolve true se a pessoa permitiu. */
async function askDockerPermission() {
  const yes = await confirmModal({
    title: t('p.dockerAskTitle'), text: t('p.dockerAskText'), yes: t('p.dockerAllow'), no: t('p.dockerNo'), danger: false,
  });
  await setDockerPermission(yes);
  return yes;
}

async function enterDockerSource() {
  if (state.dockerList === true) { await loadDocker(); return; }
  if (state.dockerList === null && await askDockerPermission()) { await loadDocker(); return; }
  renderProject();
}

async function checkTrivy() {
  try { state.trivy = await invoke('trivy_available'); } catch { state.trivy = false; }
}

async function setScan(on) {
  try { state.scanImages = (await invoke('update_settings', { scanImages: on })).scanImages; } catch { /* mantém */ }
  if (state.scanImages && state.trivy === null) await checkTrivy();
}

/** Roda o Trivy na imagem escolhida. Devolve true se pode seguir com a publicação. */
async function scanBeforePublish() {
  if (!(state.scanImages && state.trivy)) return true;
  const btn = $('#publish');
  const label = btn.textContent;
  const fromImage = state.source === 'docker';
  btn.disabled = true;
  btn.textContent = t(fromImage ? 'p.scanning' : 'p.scanBuilding');
  let report;
  try {
    report = fromImage ? await invoke('scan_image', { image: state.image }) : await invoke('scan_project', { dir: state.dir });
  } catch (e) {
    btn.textContent = label; btn.disabled = false;
    return confirmModal({ title: t('p.scanFailTitle'), text: t('p.scanFailText', { err: String(e) }), yes: t('p.scanGo'), no: t('p.scanBack'), danger: false });
  }
  btn.textContent = label; btn.disabled = false;
  if (!report.secrets_total && !report.critical && !report.high) return true;
  const lines = [];
  if (report.secrets_total) {
    lines.push(t('p.scanSecrets', { n: report.secrets_total }));
    report.secrets.forEach((x) => lines.push(`• ${x.title} — ${x.file}`));
    if (report.secrets_total > report.secrets.length) lines.push(t('p.scanMore', { n: report.secrets_total - report.secrets.length }));
  }
  if (report.critical || report.high) {
    if (lines.length) lines.push('');
    lines.push(t('p.scanVulns', { c: report.critical, h: report.high, f: report.fixable }));
    report.vulns.forEach((x) => lines.push(`• ${x.id} — ${x.package} ${x.installed}${x.fixed ? ' → ' + x.fixed : ''}`));
    const shown = report.vulns.length;
    if (report.critical + report.high > shown) lines.push(t('p.scanMore', { n: report.critical + report.high - shown }));
  }
  lines.push('', t('p.scanNote'));
  return confirmModal({ title: t('p.scanTitle'), text: lines.join('\n'), yes: t('p.scanGo'), no: t('p.scanBack'), danger: false });
}

async function loadDocker() {
  if (state.trivy === null) checkTrivy().then(renderProject);
  state.dockerLoading = true;
  state.dockerError = '';
  renderProject();
  try {
    state.docker = await invoke('list_local_docker');
  } catch (e) {
    state.docker = null;
    const msg = String(e);
    state.dockerError = msg === 'permission' ? t('p.dockerOffText') : msg;
  }
  state.dockerLoading = false;
  renderProject();
}

function dockerRow(img) {
  const row = el('div', 'dk-row');
  const ok = img.os === 'linux' && img.arch === 'amd64';
  const b = el('button', 'dimg');
  b.type = 'button';
  b.setAttribute('role', 'radio');
  b.setAttribute('aria-checked', String(state.image === img.reference));
  if (!ok) b.setAttribute('aria-disabled', 'true');
  b.dataset.ref = img.reference;
  const name = el('span', 'dimg-name');
  name.append(el('span', '', img.reference));
  if (img.running.length) name.append(el('span', 'pill', t('p.imgInUse')));
  if (!ok) name.append(el('span', 'pill warn', t('p.imgBadArch', { arch: `${img.os}/${img.arch}` })));
  const bits = [fmtSize(img.size_bytes), ago(img.created),
    img.exposed_ports.length ? t('p.imgPort', { port: img.exposed_ports.join(', ') }) : t('p.imgNoPort')];
  if (img.running.length) bits.push(t('p.imgUsedBy', { names: img.running.join(', ') }));
  const main = el('span', 'dimg-main');
  main.append(name, el('span', 'dimg-sub', bits.filter(Boolean).join(' · ')));
  b.append(main);
  row.append(b);
  const dir = projectDirFor(img);
  if (dir) {
    const open = el('button', 'btn small ghost', t('p.imgOpenDir'));
    open.type = 'button';
    open.dataset.dir = dir;
    row.append(open);
  }
  return row;
}

function renderDocker() {
  const allowed = state.dockerList === true;
  $('#dk-off').hidden = allowed;
  $('#dk-body').hidden = !allowed;
  if (!allowed) {
    $('#dk-off-text').textContent = t('p.dockerOffText');
    return;
  }
  const list = $('#dk-list');
  const msg = $('#dk-msg');
  list.textContent = '';
  $('#dk-all').checked = state.dockerAll;
  if (state.dockerLoading) { msg.textContent = t('p.dockerLoading'); msg.hidden = false; return; }
  if (state.dockerError) { msg.textContent = state.dockerError; msg.hidden = false; return; }
  const imgs = (state.docker ? state.docker.images : [])
    .filter((i) => state.dockerAll || i.running.length || i.exposed_ports.length);
  msg.textContent = t('p.dockerEmpty');
  msg.hidden = imgs.length > 0;
  imgs.forEach((i) => list.append(dockerRow(i)));
}

function renderDockerDetect(det) {
  const img = findImage(state.image);
  if (!img) { det.hidden = true; return; }
  const first = img.exposed_ports[0];
  det.textContent = (first ? t('p.imgPortOk', { ref: img.reference, port: first }) : t('p.imgPortNone', { ref: img.reference }))
    + ' ' + t('p.imgWarn');
  det.hidden = false;
}

function pickImage(ref) {
  const img = findImage(ref);
  if (!img || !(img.os === 'linux' && img.arch === 'amd64')) return;
  state.image = ref;
  const d = state.draft;
  d.name = slug(ref);
  d.port = img.exposed_ports[0] ?? '';
  d.typedName = d.typedPort = false;
  $('#app-name').value = d.name;
  $('#app-port').value = d.port;
  showError($('#project-error'), '');
  renderProject();
}

async function openProjectDir(dir) {
  state.source = 'folder';
  $('#live-banner').hidden = true;
  renderProject();
  await loadProject(dir);
}

/* ---------- origem: arquivo compactado ---------- */

async function pickArchive() {
  let path = null;
  try {
    path = await TAURI.dialog.open({
      directory: false, multiple: false, title: t('p.pickArchive'),
      filters: [{ name: t('p.archiveFilter'), extensions: ['zip', 'tar', 'gz', 'tgz', 'rar'] }],
    });
  } catch { return; }
  if (!path) return;
  path = Array.isArray(path) ? path[0] : path;
  const err = $('#project-error');
  showError(err, '');
  state.archiveName = path.split(/[\\/]/).pop();
  $('#archive-box').textContent = t('p.archiveOpening');
  $('#pick-archive').disabled = true;
  try {
    await applyProject(await invoke('extract_archive', { path }));
  } catch (e) {
    state.info = null;
    state.infoSource = null;
    showError(err, String(e));
  } finally {
    $('#pick-archive').disabled = false;
    renderProject();
  }
}

/* ---------- configurações e permissões ---------- */

function renderSettings() {
  $('#set-profiles').checked = state.readProfiles;
  $('#set-docker').checked = state.dockerList === true;
  $('#set-unmanaged').checked = state.deleteUnmanaged;
  $('#set-unmanaged-state').textContent = state.deleteUnmanaged ? t('s.scanOn') : t('s.scanOff');
  $('#set-scan').checked = state.scanImages;
  if (state.scanImages && state.trivy === null) checkTrivy().then(() => { if (state.screen === 'settings') renderSettings(); });
  $('#set-scan-state').textContent = !state.scanImages ? t('s.scanOff') : state.trivy === false ? t('s.scanNoTool') : t('s.scanOn');
  $('#set-docker-state').textContent = state.dockerList === true ? t('s.dockerOn')
    : state.dockerList === false ? t('s.dockerOff') : t('s.dockerNever');
}

function openSettings() {
  if (state.screen === 'settings') return;
  state.settingsReturn = state.screen;
  showScreen('settings');
}

/* ---------- 3. deploy ---------- */

function fmtElapsed(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** Cronômetro do deploy: mostrado nos detalhes técnicos e ao lado da etapa em andamento. */
function tickTimer() {
  if (!state.deployStart) { $('#log-timer').textContent = ''; return; }
  const end = state.deploying ? Date.now() : state.deployEnd;
  $('#log-timer').textContent = '· ' + fmtElapsed(end - state.deployStart);
  if (state.screen === 'deploy' && state.deploying) renderTasks();
}

function stopTimer() {
  clearInterval(state.timerId);
  state.timerId = null;
  state.deployEnd = Date.now();
  tickTimer();
}

function resetDeploy() {
  clearInterval(state.timerId);
  state.timerId = null;
  state.deployStart = null;
  state.deployEnd = null;
  $('#log-timer').textContent = '';
  state.stageState = {};
  state.stageStart = {};
  state.stageSecs = {};
  STAGES.forEach((s) => { state.stageState[s] = 'todo'; });
  state.logLines = [];
  state.deployDone = false;
  state.deployFailed = false;
  state.deployCancelled = false;
  $('#log').textContent = '';
  showError($('#deploy-error'), '');
  $('#deploy-error').hidden = true;
}

async function startDeploy() {
  resetDeploy();
  state.deploying = true;
  state.deployStart = Date.now();
  state.timerId = setInterval(tickTimer, 1000);
  tickTimer();
  showScreen('deploy');
  try {
    await invoke('start_deploy', { dir: state.dir, profile: state.profile });
  } catch (e) {
    failDeploy(String(e));
  }
}

function renderTasks() {
  const check = '<svg width="10" height="10" viewBox="0 0 12 12" aria-hidden="true"><path d="M2.5 6.5l2.3 2.3L9.5 3.5"/></svg>';
  $('#tasks').innerHTML = STAGES.map((s) => {
    const st = state.stageState[s] || 'todo';
    let note = '';
    if (st === 'done' && state.stageSecs[s] != null) note = state.stageSecs[s] + ' s';
    else if (st === 'current') {
      const hint = t('d.hint.' + s) === 'd.hint.' + s ? '' : t('d.hint.' + s);
      const spent = state.stageStart[s] ? fmtElapsed(Date.now() - state.stageStart[s]) : '';
      note = [hint, spent].filter(Boolean).join(' · ');
    }
    return `<li class="task ${st}"><span class="ico">${st === 'done' ? check : ''}</span><span class="label">${t('d.stage.' + s)}</span><span class="note">${note}</span></li>`;
  }).join('');
}

function renderDeploy() {
  const name = state.draft.name || (state.info && state.info.suggested_name) || '';
  $('#deploy-title').textContent = t('d.title', { name });
  $('#deploy-intro').textContent = state.deployDone ? t('d.finished') : t('d.intro');
  renderTasks();
  const running = state.deploying;
  $('#deploy-cancel').hidden = !running;
  $('#deploy-back').hidden = running;
  $('#deploy-retry').hidden = !(state.deployFailed || state.deployCancelled);
  renderSteps(); // o menu destrava quando o deploy termina, falha ou é cancelado
}

function appendLog(line) {
  const clean = String(line).replace(ANSI_RE, '');
  state.logLines.push(clean);
  const el = $('#log');
  if (state.logLines.length > MAX_LOG_LINES) {
    state.logLines = state.logLines.slice(-MAX_LOG_LINES);
    el.textContent = state.logLines.join('\n') + '\n';
  } else {
    el.textContent += clean + '\n';
  }
  el.scrollTop = el.scrollHeight;
}

function failDeploy(message) {
  state.deploying = false;
  stopTimer();
  state.deployFailed = true;
  const cur = STAGES.find((s) => state.stageState[s] === 'current');
  if (cur) state.stageState[cur] = 'failed';
  const box = $('#deploy-error');
  box.textContent = `${t('d.failed')}:\n${message}`;
  box.hidden = false;
  renderDeploy();
}

function finishDeploy(url) {
  state.deploying = false;
  state.existing = null;
  stopTimer();
  state.deployDone = true;
  state.url = url || null;
  state.deployedAt = Date.now();
  STAGES.forEach((s) => { if (state.stageState[s] !== 'done') state.stageState[s] = 'done'; });
  showScreen('live');
  loadApps();
}

function onDeployEvent(ev) {
  switch (ev.type) {
    case 'stage_started':
      state.stageState[ev.stage] = 'current';
      state.stageStart[ev.stage] = Date.now();
      if (state.screen === 'deploy') renderTasks();
      break;
    case 'stage_done':
      state.stageState[ev.stage] = 'done';
      if (state.stageStart[ev.stage]) state.stageSecs[ev.stage] = Math.round((Date.now() - state.stageStart[ev.stage]) / 1000);
      if (state.screen === 'deploy') renderTasks();
      break;
    case 'log':
      appendLog(ev.line);
      break;
    case 'done':
      finishDeploy(ev.url);
      break;
    case 'failed':
      failDeploy(ev.message);
      break;
    case 'cancelled':
      state.deploying = false;
      stopTimer();
      state.deployCancelled = true;
      {
        const box = $('#deploy-error');
        box.textContent = t('d.cancelled');
        box.hidden = false;
      }
      renderDeploy();
      break;
    default:
      break;
  }
}

/* ---------- 4. no ar ---------- */

function renderLive() {
  const d = state.draft;
  $('#live-current').hidden = !state.url;
  fillAppsRegions();
  renderAppsHeader();
  const name = d.name || (state.info && state.info.suggested_name) || '';
  $('#live-title').textContent = t('l.title', { name });
  $('#live-url').textContent = state.url || '—';
  $('#open-url').disabled = !state.url;
  $('#copy-url').disabled = !state.url;
  $('#st-dest').textContent = `${d.target === 'ec2' ? 'EC2' : 'Lightsail'} · ${d.size}`;
  $('#st-region').textContent = regionLabel(d.region);
  let ver = t('l.unknown');
  if (state.deployedAt) {
    const min = Math.floor((Date.now() - state.deployedAt) / 60000);
    ver = min < 1 ? t('l.now') : t('l.min', { n: min });
  }
  $('#st-version').textContent = ver;
  renderApps();
}

/* ---------- apps na conta (lista do passo "No ar") ---------- */

let appsSeq = 0;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function fillAppsRegions() {
  const sel = $('#apps-region');
  const keep = state.appsRegion || sel.value || state.draft.region || DEFAULT_REGION;
  sel.innerHTML = state.regions.map((r) => `<option value="${r.code}">${r.code} (${r.name})</option>`).join('');
  sel.value = keep;
  if (!sel.value && state.regions.length) sel.value = state.regions[0].code;
}

function renderAppsHeader() {
  const title = $('#apps-title');
  title.textContent = state.url ? t('a.title') : t('a.pageTitle');
  title.classList.toggle('sub', Boolean(state.url));
}

/** Estados em que o app ainda está sendo criado, atualizado ou parado: vale reler a lista. */
const TRANSIENT_STATE = /^(PENDING|DEPLOYING|CREATING|UPDATING|STOPPING|STARTING|SHUTTING-DOWN)$/i;
const APPS_POLL_MS = 10000;
const APPS_POLL_MAX = 120; // 20 minutos: depois disso o botão Atualizar continua valendo

let appsTimer = null;
let appsPolls = 0;

/** Enquanto algum app estiver em transição e a tela de Apps aberta, relê a lista de tempos em tempos. */
function scheduleAppsRefresh() {
  clearTimeout(appsTimer);
  appsTimer = null;
  const apps = state.apps && state.apps.res.apps;
  const busy = (a) => TRANSIENT_STATE.test(a.state) || a.checks === 'initializing';
  if (state.screen !== 'live' || !apps || !apps.some(busy)) { appsPolls = 0; return; }
  if (appsPolls >= APPS_POLL_MAX) return;
  appsTimer = setTimeout(() => {
    appsTimer = null;
    if (state.screen !== 'live') return;
    appsPolls += 1;
    loadApps({ silent: true });
  }, APPS_POLL_MS);
}

async function loadApps({ silent = false } = {}) {
  if (!state.account) return;
  if (!silent) appsPolls = 0;
  fillAppsRegions();
  if (!silent) showError($('#apps-error'), '');
  const seq = ++appsSeq;
  const region = $('#apps-region').value || DEFAULT_REGION;
  state.appsRegion = region;
  if (!silent) {
    state.apps = null;
    renderApps();
  }
  try {
    const res = await invoke('list_apps', { profile: state.profile, region });
    if (seq !== appsSeq) return;
    state.apps = { region, res };
  } catch (e) {
    if (seq !== appsSeq) return;
    if (silent && state.apps) { scheduleAppsRefresh(); return; } // falha de rede na releitura: mantém a lista e tenta de novo
    state.apps = { region, res: { apps: [], warnings: [String(e)] } };
  }
  renderApps();
  scheduleAppsRefresh();
}

function appCard(app) {
  const card = el('div', 'card app-card');
  const main = el('div', 'app-main');
  const name = el('div', 'app-name');
  name.append(el('b', '', app.name), el('span', 'pill', app.target === 'ec2' ? 'EC2' : 'Lightsail'));
  if (app.managed) { const m = el('span', 'pill', t('a.managed')); m.title = t('a.managedTip'); name.append(m); }
  const sub = el('div', 'app-sub');
  const ok = /^(READY|RUNNING|ACTIVE)$/i.test(app.state);
  const wait = TRANSIENT_STATE.test(app.state);
  sub.append(el('span', 'state' + (ok ? ' ok' : wait ? ' wait' : ''), app.state));
  if (app.checks) {
    // status checks da instância (EC2): "running" não quer dizer que já passou nas verificações
    const bad = app.checks === 'impaired';
    const good = app.checks === 'ok';
    const chk = el('span', 'state' + (good ? ' ok' : bad ? ' bad' : ' wait'), t('a.checks.' + app.checks));
    chk.title = t('a.checksTip');
    sub.append(chk);
  }
  sub.append(el('span', 'mono', app.url || t('a.noUrl')));
  main.append(name, sub);
  card.append(main);
  if (app.url) {
    const copy = el('button', 'btn small icon');
    copy.type = 'button';
    copy.dataset.copy = app.url;
    copy.title = t('a.copy');
    copy.setAttribute('aria-label', t('a.copy'));
    copy.innerHTML = COPY_ICON;
    const open = el('button', 'btn small');
    open.type = 'button';
    open.dataset.open = app.url;
    open.title = t('ext');
    open.append(el('span', '', t('a.open')));
    open.insertAdjacentHTML('beforeend', EXT_ICON);
    card.append(copy, open);
  }
  const rm = el('button', 'btn small ghost danger-text', t('a.remove'));
  rm.type = 'button';
  rm.dataset.removeName = app.name;
  rm.dataset.removeTarget = app.target;
  rm.dataset.removeRegion = app.region;
  rm.dataset.removeManaged = String(app.managed);
  if (!app.managed && !state.deleteUnmanaged) { rm.disabled = true; rm.title = t('a.removeLocked'); }
  card.append(rm);
  return card;
}

function renderApps() {
  const list = $('#apps-list');
  const warn = $('#apps-warn');
  const status = $('#apps-status');
  list.textContent = '';
  const a = state.apps;
  if (!a) {
    warn.hidden = true;
    status.textContent = state.account || state.checking ? t('a.loading') : t('a.needAccount');
    status.hidden = false;
    return;
  }
  const w = a.res.warnings || [];
  if (w.length) {
    // Resumo curto na tela; o texto completo do erro fica atrás de "Mostrar mais".
    const isPerm = (x) => /Unauthorized|AccessDenied|not authorized/i.test(x);
    const denied = [...new Set(w.filter(isPerm).map((x) => x.split(':')[0].trim()))];
    const summary = w.every(isPerm) && denied.length ? t('a.warnPerm', { svc: denied.join(', ') }) : t('a.warn');
    $('#apps-warn-text').textContent = summary;
    $('#apps-warn-detail').textContent = w.join('\n\n');
    $('#apps-warn-detail').hidden = !state.warnOpen;
    $('#apps-warn-toggle').textContent = t(state.warnOpen ? 'a.less' : 'a.more');
    $('#apps-warn-toggle').setAttribute('aria-expanded', String(state.warnOpen));
    warn.hidden = false;
  } else {
    warn.hidden = true;
  }
  status.textContent = t('a.empty', { region: a.region });
  status.hidden = a.res.apps.length > 0;
  a.res.apps.forEach((app) => list.appendChild(appCard(app)));
}

/** Remove um app da lista da conta (funciona mesmo sem uma pasta de projeto aberta). */
async function removeListedApp(name, target, region, managed) {
  const warn = managed ? '' : `${t('a.removeUnmanagedWarn')}\n\n`;
  const yes = await confirmModal({
    title: t('l.confirmTitle', { name }),
    text: `${name} (${target === 'ec2' ? 'EC2' : 'Lightsail'}, ${regionLabel(region)})\n\n${warn}${t('l.confirmText')}`,
    yes: t('l.confirmYes'),
    no: t('l.confirmNo'),
  });
  if (!yes) return;
  try {
    await invoke('destroy_app', { profile: state.profile, region, target, name });
    state.existing = null;
    if (state.draft.name === name) { state.url = null; state.deployedAt = null; }
  } catch (e) {
    showError($('#apps-error'), String(e) === 'unmanaged' ? t('a.removeUnmanagedErr') : t('a.removeFailed', { name, err: String(e) }));
    return;
  }
  await loadApps();
  renderLive();
}

async function removeApp() {
  const name = state.draft.name;
  const yes = await confirmModal({
    title: t('l.confirmTitle', { name }),
    text: t('l.confirmText'),
    yes: t('l.confirmYes'),
    no: t('l.confirmNo'),
  });
  if (!yes) return;
  const btn = $('#remove');
  btn.disabled = true;
  btn.textContent = t('l.removing');
  try {
    await invoke('destroy', { dir: state.dir, profile: state.profile });
    state.existing = null;
    state.url = null;
    state.deployedAt = null;
    state.info = null;
    state.dir = null;
    await enterProject();
    $('#live-banner-text').textContent = t('l.removed', { name });
    $('#live-banner-go').hidden = true;
    $('#live-banner').hidden = false;
  } catch (e) {
    showError($('#project-error'), String(e));
    showScreen('project');
  } finally {
    btn.disabled = false;
    btn.textContent = t('l.removeBtn');
  }
}

/* ---------- eventos ---------- */

function wire() {
  // menu de configurações
  $('#gear').addEventListener('click', (e) => { e.stopPropagation(); toggleMenu(); });
  $('#menu').addEventListener('click', (e) => {
    const b = e.target.closest('.menu-item');
    if (!b) return;
    if (b.dataset.lang) setLang(b.dataset.lang);
    if (b.dataset.theme) setTheme(b.dataset.theme);
    toggleMenu(false);
    if (b.dataset.action === 'terms') openTerms(true);
    if (b.dataset.action === 'settings') openSettings();
  });
  document.addEventListener('click', (e) => {
    if (!e.target.closest('.side-foot')) { toggleMenu(false); togglePMenu(false); }
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') { toggleMenu(false); togglePMenu(false); } });

  // conectar
  $('#copy-policy').addEventListener('click', async (e) => copyText(await invoke('policy', { target: 'lightsail' }), e.currentTarget));
  $('#copy-policy-ec2').addEventListener('click', async (e) => copyText(await invoke('policy', { target: 'ec2' }), e.currentTarget));
  $('#open-iam').addEventListener('click', () => openUrl('https://console.aws.amazon.com/iam/home#/users'));
  $('#connect-go').addEventListener('click', connectWithKeys);
  $('#secret').addEventListener('keydown', (e) => { if (e.key === 'Enter') connectWithKeys(); });
  $('#pmenu').addEventListener('click', (e) => {
    // o menu é redesenhado a cada clique; sem isto o alvo sai do DOM e o clique global fecha o menu
    e.stopPropagation();
    const b = e.target.closest('.menu-item');
    if (!b) return;
    if (b.dataset.profile) { switchProfile(b.dataset.profile); return; }
    togglePMenu(false);
    if (b.dataset.action === 'keys') goStep('connect');
    if (b.dataset.action === 'terms') openTerms(true);
  });

  // termos de uso e rodapé (volta para a conta/perfis)
  $('#terms-accept').addEventListener('change', () => { $('#terms-go').disabled = !$('#terms-accept').checked; });
  $('#terms-go').addEventListener('click', acceptTerms);
  $('#terms-back').addEventListener('click', () => { state.termsReadOnly = false; showScreen(state.prevScreen || 'connect'); });
  $('#foot-text').addEventListener('click', (e) => { e.stopPropagation(); if (state.termsOk) togglePMenu(); });

  // projeto
  $('#pick-dir').addEventListener('click', pickFolder);
  $$('.choice[data-target]').forEach((c) => c.addEventListener('click', () => {
    readProjectForm();
    state.draft.target = c.dataset.target;
    state.draft.size = sizesFor(state.draft.target)[0];
    renderProject();
  }));
  $$('.choice[data-source]').forEach((c) => c.addEventListener('click', () => setSource(c.dataset.source)));
  $('#pick-archive').addEventListener('click', pickArchive);
  $('#dk-refresh').addEventListener('click', loadDocker);
  $('#dk-scan').addEventListener('change', async (e) => { await setScan(e.target.checked); renderProject(); });
  $('#dk-all').addEventListener('change', (e) => { state.dockerAll = e.target.checked; renderProject(); });
  $('#dk-allow').addEventListener('click', async () => { if (await askDockerPermission()) await loadDocker(); else renderProject(); });
  $('#dk-settings').addEventListener('click', openSettings);
  $('#dk-list').addEventListener('click', (e) => {
    const dir = e.target.closest('[data-dir]');
    if (dir) { openProjectDir(dir.dataset.dir); return; }
    const b = e.target.closest('.dimg[data-ref]');
    if (b && b.getAttribute('aria-disabled') !== 'true') pickImage(b.dataset.ref);
  });

  // configurações
  $('#set-profiles').addEventListener('change', async (e) => {
    try { state.readProfiles = (await invoke('update_settings', { readProfiles: e.target.checked })).readProfiles; } catch { /* mantém */ }
    state.profiles = null;
    renderSettings();
  });
  $('#set-docker').addEventListener('change', async (e) => {
    try { await setDockerPermission(e.target.checked); } catch { /* mantém */ }
    renderSettings();
  });
  $('#set-scan').addEventListener('change', async (e) => { await setScan(e.target.checked); renderSettings(); });
  $('#set-unmanaged').addEventListener('change', async (e) => {
    let on = e.target.checked;
    if (on && !(await confirmModal({ title: t('s.unmanagedAskTitle'), text: t('s.unmanagedAskText'), yes: t('p.dockerAllow'), no: t('p.dockerNo') }))) on = false;
    try { state.deleteUnmanaged = (await invoke('update_settings', { deleteUnmanaged: on })).deleteUnmanaged; } catch { /* mantém */ }
    renderSettings();
  });
  $('#set-back').addEventListener('click', () => showScreen(state.settingsReturn || 'connect'));
  $('#set-terms').addEventListener('click', () => openTerms(true));
  ['#region', '#size'].forEach((sel) => $(sel).addEventListener('change', () => { readProjectForm(); loadExisting(); renderUpdateNote(); }));
  $('#app-name').addEventListener('input', () => { state.draft.typedName = true; readProjectForm(); renderUpdateNote(); });
  $('#app-port').addEventListener('input', () => { state.draft.typedPort = true; });
  $('#publish').addEventListener('click', publish);

  // deploy
  $('#deploy-cancel').addEventListener('click', () => invoke('cancel_deploy'));
  $('#deploy-back').addEventListener('click', () => {
    if (state.deployDone && state.url) showScreen('live');
    else showScreen('project');
  });
  $('#deploy-retry').addEventListener('click', startDeploy);
  $('#copy-log').addEventListener('click', (e) => copyText(state.logLines.join('\n'), e.currentTarget));

  // no ar
  $('#copy-url').addEventListener('click', (e) => copyText(state.url, e.currentTarget));
  $('#open-url').addEventListener('click', (e) => state.url && openUrl(state.url, e.currentTarget));
  $('#new-version').addEventListener('click', startDeploy);
  $('#tech-details').addEventListener('click', () => { state.deploying = false; showScreen('deploy'); });
  $('#other-project').addEventListener('click', () => {
    state.info = null;
    state.dir = null;
    $('#live-banner').hidden = true;
    showError($('#project-error'), '');
    showScreen('project');
  });
  $('#remove').addEventListener('click', removeApp);

  // passos da barra lateral e lista de apps
  $('#steps').addEventListener('click', (e) => {
    const b = e.target.closest('.step');
    if (b && !b.disabled) goStep(b.dataset.nav);
  });
  $('#apps-refresh').addEventListener('click', loadApps);
  $('#apps-warn-toggle').addEventListener('click', () => { state.warnOpen = !state.warnOpen; renderApps(); });
  $('#apps-region').addEventListener('change', () => { state.appsRegion = $('#apps-region').value; loadApps(); });
  $('#apps-list').addEventListener('click', (e) => {
    const c = e.target.closest('[data-copy]');
    if (c) { copyText(c.dataset.copy, c); return; }
    const rm = e.target.closest('[data-remove-name]');
    if (rm) { if (!rm.disabled) removeListedApp(rm.dataset.removeName, rm.dataset.removeTarget, rm.dataset.removeRegion, rm.dataset.removeManaged === 'true'); return; }
    const o = e.target.closest('[data-open]');
    if (o) openUrl(o.dataset.open, o);
  });
}

async function init() {
  document.documentElement.dataset.theme = state.theme;
  state.termsOk = store.get('terms') === TERMS_VERSION;
  resetDeploy();
  wire();
  await loadRegions();
  try {
    // as permissões ficam no backend (arquivo de configuração), que é quem as confere
    const s = await invoke('get_settings');
    state.readProfiles = s.readProfiles;
    state.dockerList = s.dockerList;
    state.scanImages = s.scanImages;
    state.deleteUnmanaged = s.deleteUnmanaged;
  } catch { /* usa os padrões */ }
  await TAURI.event.listen('deploy-event', (e) => onDeployEvent(e.payload));
  applyI18n();
  if (!state.termsOk) {
    // primeira vez (ou os termos mudaram): nada é lido nem chamado antes do aceite
    state.termsReadOnly = false;
    $('#terms-accept').checked = false;
    $('#terms-profiles').checked = state.readProfiles;
    showScreen('terms');
    return;
  }
  await startAfterTerms();
}

init();
