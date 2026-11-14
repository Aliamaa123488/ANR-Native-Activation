const invoke = window.__TAURI__?.core?.invoke;
const icons = {
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  folder: '<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h5l2 2h8A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"/><path d="M3 10h18"/>',
  gpu: '<rect x="4" y="5" width="16" height="14" rx="2"/><path d="M8 9h8v6H8zM8 2v3m4-3v3m4-3v3M8 19v3m4-3v3m4-3v3M2 9h2m-2 4h2m16-4h2m-2 4h2"/>',
  moon: '<path d="M20.4 15.2A8.5 8.5 0 0 1 8.8 3.6 8.5 8.5 0 1 0 20.4 15.2Z"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M2 12h2m16 0h2M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42"/>',
  play: '<path d="m8 5 11 7-11 7z" fill="currentColor" stroke="none"/>',
  download: '<path d="M12 3v12m-5-5 5 5 5-5M4 20h16"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5"/><path d="M5.6 9a7 7 0 0 1 11.6-2L20 12M4 12l2.8 5a7 7 0 0 0 11.6-2"/>',
  shield: '<path d="M12 22s8-4 8-11V5l-8-3-8 3v6c0 7 8 11 8 11Z"/><path d="m9 12 2 2 4-4"/>',
};
const icon = (name, cls = '') => `<svg class="icon ${cls}" viewBox="0 0 24 24" aria-hidden="true">${icons[name]}</svg>`;

const state = {
  folder: localStorage.getItem('game-folder') || '',
  executable: '',
  executables: [],
  hasOptiscaler: false,
  installed: false,
  gpus: [],
  runtime: null,
  selectedGpu: localStorage.getItem('selected-gpu') || 'auto',
  release: null,
  busy: false,
  theme: localStorage.getItem('theme') || 'dark',
};

document.documentElement.dataset.theme = state.theme;

const monitorPreferenceDefaults = {
  fps: true,
  onePercentLow: true,
  pointOnePercentLow: true,
  gpuUsage: true,
  cpuUsage: true,
  overlayAfterLaunch: false,
};
try {
  state.monitorPreferences = {
    ...monitorPreferenceDefaults,
    ...JSON.parse(localStorage.getItem('monitor-preferences') || '{}'),
  };
} catch {
  state.monitorPreferences = { ...monitorPreferenceDefaults };
}

const isOverlay = new URLSearchParams(window.location.search).has('overlay');
if (isOverlay) {
  document.documentElement.classList.add('overlay-window');
  document.querySelector('#app').innerHTML = `
    <section class="overlay-panel" aria-live="polite">
      <div class="overlay-heading"><span class="overlay-indicator"></span><strong id="overlay-game">ANR MONITOR</strong><span class="overlay-live">LIVE</span></div>
      <div class="overlay-metrics" id="overlay-metrics"></div>
    </section>
  `;
  const renderOverlay = async () => {
    try {
      const snapshot = await invoke('get_monitor_snapshot');
      const { preferences: prefs } = snapshot;
      const readings = [
        ['fps', 'FPS', snapshot.fps, 'fps'],
        ['onePercentLow', '1% LOW', snapshot.onePercentLow, 'fps'],
        ['pointOnePercentLow', '0.1% LOW', snapshot.pointOnePercentLow, 'fps'],
        ['gpuUsage', 'GPU', snapshot.gpuUsage, 'percent'],
        ['cpuUsage', 'CPU', snapshot.cpuUsage, 'percent'],
      ].filter(([key]) => prefs[key]);
      $('#overlay-game').textContent = snapshot.gameRunning ? snapshot.gameName : 'ANR MONITOR';
      $('#overlay-metrics').innerHTML = readings.map(([key, label, value, kind]) => {
        const warming = value == null;
        const numericValue = value == null ? null : Math.round(value);
        const colorClass = kind === 'percent'
          ? numericValue >= 85 ? 'hot' : numericValue >= 60 ? 'warm' : 'cool'
          : numericValue >= 60 ? 'cool' : numericValue >= 30 ? 'warm' : 'hot';
        const shown = warming ? (snapshot.gameRunning ? '…' : '—') : `${numericValue}${kind === 'percent' ? '%' : ''}`;
        return `<div class="overlay-reading"><span>${label}</span><strong class="${warming ? '' : colorClass}">${shown}</strong></div>`;
      }).join('') || '<div class="overlay-empty">Choose metrics in ANR Monitoring</div>';
    } catch {
      $('#overlay-game').textContent = 'ANR MONITOR';
    }
  };
  const $ = (selector) => document.querySelector(selector);
  renderOverlay();
  window.setInterval(renderOverlay, 700);
} else {

document.querySelector('#app').innerHTML = `
  <div class="shell">
    <header class="topbar">
      <a class="brand" href="#" aria-label="ANR Native Activation home">
        <span class="brand-mark">A<span>✳</span></span>
        <span class="brand-name">APEX<span>COMPUTE</span></span>
      </a>
      <div class="topbar-right">
        <div class="beta-pill"><span></span> FRAME GENERATION BETA</div>
        <button class="theme-button" id="theme-toggle" aria-label="Switch color theme"></button>
      </div>
    </header>

    <nav class="view-tabs" aria-label="App sections" role="tablist">
      <button class="view-tab is-active" id="setup-tab" role="tab" aria-selected="true" aria-controls="launcher-view">Game setup</button>
      <button class="view-tab" id="monitor-tab" role="tab" aria-selected="false" aria-controls="monitor-view">Monitoring</button>
    </nav>

    <main id="launcher-view">
      <section class="hero">
        <div class="hero-copy">
          <div class="eyebrow"><span class="eyebrow-line"></span> APEX NEURAL RENDERING</div>
          <h1>More frames.<br /><em>Less setup.</em></h1>
          <p>Install ANR FG into a game and launch it with your graphics preference in a few clicks.</p>
        </div>
        <div class="hero-art" aria-hidden="true">
          <div class="orbit orbit-one"></div><div class="orbit orbit-two"></div>
          <div class="core"><span>A</span><i></i></div>
          <span class="spark spark-a">✳</span><span class="spark spark-b">✳</span><span class="spark spark-c">✳</span>
        </div>
      </section>

      <section class="release-strip" aria-live="polite">
        <div class="release-icon">${icon('download')}</div>
        <div class="release-copy"><strong id="release-title">Checking latest ANR release…</strong><span id="release-subtitle">GitHub release packages are cached in Documents</span></div>
        <div class="release-right"><span class="release-dot"></span><span id="release-state">CONNECTING</span><button class="icon-button" id="refresh-release" title="Check for updates" aria-label="Check for updates">${icon('refresh')}</button></div>
      </section>

      <div class="section-heading"><span>01</span><h2>Set up your game</h2><div class="heading-rule"></div><span class="optional">ABOUT A MINUTE</span></div>

      <section class="setup-card">
        <div class="field-label-row"><label for="game-path">GAME FOLDER</label><span>Choose the folder that contains the game’s .exe</span></div>
        <div class="path-row">
          <div class="path-icon">${icon('folder')}</div>
          <div class="path-value" id="path-value">Select a game folder to get started</div>
          <button class="browse-button" id="browse">Browse <span>⌘</span></button>
        </div>
        <div class="field-label-row executable-label"><label for="executable">GAME EXECUTABLE</label><span id="exe-help">Detected in the selected folder</span></div>
        <select id="executable" class="select-control" disabled><option value="">Choose a game folder first</option></select>
      </section>

      <div class="section-heading gpu-heading"><span>02</span><h2>Choose a graphics adapter</h2><div class="heading-rule"></div></div>
      <section class="gpu-card">
        <div class="gpu-heading-icon">${icon('gpu')}</div>
        <div class="gpu-content">
          <div class="field-label-row"><label for="gpu-select">PREFERRED GPU</label><span id="gpu-caption">Select any detected integrated or discrete adapter</span></div>
          <select id="gpu-select" class="select-control"><option value="auto">Auto — let Windows decide</option></select>
          <div class="gpu-note">${icon('shield')} Windows applies the power-saving or high-performance category; it chooses the exact card if several share that category.</div>
        </div>
      </section>

      <section class="compat-row">
        <div class="compat-icon">✳</div>
        <div><strong>ANR FG works with your supported GPU</strong><span>Frame Generation must also be enabled in the game’s graphics settings.</span></div>
        <span class="compat-pill" id="install-status">READY TO INSTALL</span>
      </section>
      <div class="runtime-warning" id="runtime-warning" hidden><span>Microsoft Visual C++ 2015–2022 x64 is needed by the full package.</span><button id="runtime-download">Get runtime</button></div>

      <div class="actions-row">
        <button class="install-button" id="install" disabled><span class="button-icon">${icon('download')}</span><span id="install-label">Install ANR</span>${icon('arrow', 'button-arrow')}</button>
        <button class="launch-button" id="launch" disabled>${icon('play')} Launch game</button>
        <button class="uninstall-button" id="uninstall" hidden>Uninstall</button>
      </div>
      <div class="status-message" id="status-message" role="status" aria-live="polite"></div>
      <footer><span>ANR FG is beta software from Apex Compute.</span><span>Files cached in <b>Documents / ANR Native Act</b></span></footer>
    </main>

    <main id="monitor-view" hidden>
      <div class="monitor-page-heading"><div><div class="eyebrow"><span class="eyebrow-line"></span> LIVE PERFORMANCE</div><h2>Monitoring</h2><p>Choose what you want to see. Start the game from Setup to collect live readings.</p></div><span class="monitor-state" id="monitor-state">NOT RUNNING</span></div>
      <section class="monitor-summary" aria-label="Live readings">
        <article class="monitor-stat"><span>FPS</span><strong id="fps-value">—</strong></article>
        <article class="monitor-stat"><span>1% LOW</span><strong id="one-low-value">—</strong></article>
        <article class="monitor-stat"><span>0.1% LOW</span><strong id="point-one-low-value">—</strong></article>
        <article class="monitor-stat usage-stat"><span>GPU USAGE</span><strong id="gpu-value">—</strong><i id="gpu-bar"></i></article>
        <article class="monitor-stat usage-stat"><span>CPU USAGE</span><strong id="cpu-value">—</strong><i id="cpu-bar"></i></article>
      </section>
      <section class="monitor-settings">
        <div class="monitor-section-title"><div><strong>Overlay metrics</strong><span>Pick the readings that appear over your game</span></div><span>Click-through · always on top</span></div>
        <div class="metric-options">
          <label class="metric-option"><input type="checkbox" data-metric="fps"><span class="metric-check"></span><span><b>FPS</b><small>Frames per second</small></span></label>
          <label class="metric-option"><input type="checkbox" data-metric="onePercentLow"><span class="metric-check"></span><span><b>1% low</b><small>Slowest one percent</small></span></label>
          <label class="metric-option"><input type="checkbox" data-metric="pointOnePercentLow"><span class="metric-check"></span><span><b>0.1% low</b><small>Slowest tenth of a percent</small></span></label>
          <label class="metric-option"><input type="checkbox" data-metric="gpuUsage"><span class="metric-check"></span><span><b>GPU usage</b><small>Game GPU busy time</small></span></label>
          <label class="metric-option"><input type="checkbox" data-metric="cpuUsage"><span class="metric-check"></span><span><b>CPU usage</b><small>Overall processor load</small></span></label>
          <label class="metric-option overlay-auto-option"><input type="checkbox" data-metric="overlayAfterLaunch"><span class="metric-check"></span><span><b>Open overlay on launch</b><small>Show it automatically in game</small></span></label>
        </div>
        <div class="monitor-actions"><button class="overlay-toggle" id="overlay-toggle" disabled>Show in-game overlay</button><button class="monitor-stop" id="monitor-stop" disabled>Stop monitoring</button><span id="monitor-message">Launch a game from Setup to start monitoring.</span></div>
        <div class="monitor-legend"><span><i class="legend-dot good"></i>Low</span><span><i class="legend-dot medium"></i>Moderate</span><span><i class="legend-dot high"></i>High</span><small>CPU and GPU load colors update as you play.</small></div>
      </section>
      <div class="monitor-footnote">Frame timing comes from PresentMon. The first 0.1% low appears after about 1,000 frames; both lows use a rolling 60-second window.</div>
    </main>
  </div>
`;

const $ = (selector) => document.querySelector(selector);
const pathValue = $('#path-value');
const exeSelect = $('#executable');
const gpuSelect = $('#gpu-select');
const installButton = $('#install');
const launchButton = $('#launch');
const uninstallButton = $('#uninstall');
const statusMessage = $('#status-message');

function setView(view) {
  const monitoring = view === 'monitor';
  $('#launcher-view').hidden = monitoring;
  $('#monitor-view').hidden = !monitoring;
  $('#setup-tab').classList.toggle('is-active', !monitoring);
  $('#monitor-tab').classList.toggle('is-active', monitoring);
  $('#setup-tab').setAttribute('aria-selected', String(!monitoring));
  $('#monitor-tab').setAttribute('aria-selected', String(monitoring));
}

function renderMonitor(snapshot) {
  state.monitorSnapshot = snapshot;
  state.overlayVisible = snapshot.overlayVisible;
  const reading = (value, { percent = false, decimals = 0 } = {}) => value == null ? '—' : `${value.toFixed(decimals)}${percent ? '%' : ''}`;
  $('#fps-value').textContent = reading(snapshot.fps, { decimals: 0 });
  $('#one-low-value').textContent = reading(snapshot.onePercentLow, { decimals: 0 });
  $('#point-one-low-value').textContent = reading(snapshot.pointOnePercentLow, { decimals: 0 });
  $('#gpu-value').textContent = reading(snapshot.gpuUsage, { percent: true });
  $('#cpu-value').textContent = reading(snapshot.cpuUsage, { percent: true });
  for (const key of ['gpuUsage', 'cpuUsage']) {
    const value = snapshot[key];
    const element = $(`#${key === 'gpuUsage' ? 'gpu' : 'cpu'}-value`);
    const bar = $(`#${key === 'gpuUsage' ? 'gpu' : 'cpu'}-bar`);
    const level = value == null ? 'neutral' : value >= 85 ? 'high' : value >= 60 ? 'medium' : 'good';
    element.dataset.level = level;
    bar.dataset.level = level;
    bar.style.width = `${value == null ? 0 : Math.max(0, Math.min(100, value))}%`;
  }
  const running = snapshot.gameRunning;
  $('#monitor-state').textContent = running ? 'LIVE' : 'NOT RUNNING';
  $('#monitor-state').classList.toggle('is-live', running);
  $('#monitor-message').textContent = snapshot.message || (running ? 'Collecting game performance…' : 'Launch a game from Setup to start monitoring.');
  $('#overlay-toggle').disabled = !running;
  $('#overlay-toggle').textContent = state.overlayVisible ? 'Hide in-game overlay' : 'Show in-game overlay';
  $('#monitor-stop').disabled = !running;
}

async function refreshMonitor() {
  try {
    renderMonitor(await invoke('get_monitor_snapshot'));
  } catch (error) {
    $('#monitor-message').textContent = String(error);
  }
}

function saveMonitorPreferences() {
  localStorage.setItem('monitor-preferences', JSON.stringify(state.monitorPreferences));
  invoke('set_monitor_preferences', { preferences: state.monitorPreferences }).catch(() => {});
}

function setTheme(theme) {
  state.theme = theme;
  document.documentElement.dataset.theme = theme;
  localStorage.setItem('theme', theme);
  $('#theme-toggle').innerHTML = `${icon(theme === 'dark' ? 'sun' : 'moon')}<span>${theme === 'dark' ? 'Light' : 'Dark'}</span>`;
}

function setBusy(value, message = '') {
  state.busy = value;
  document.body.classList.toggle('busy', value);
  $('#browse').disabled = value;
  $('#refresh-release').disabled = value;
  $('#theme-toggle').disabled = value;
  installButton.disabled = value || !state.folder || !state.executable;
  launchButton.disabled = value || !state.folder || !state.executable || !state.installed;
  uninstallButton.disabled = value;
  if (message) showMessage(message, 'working');
}

function showMessage(message, kind = '') {
  statusMessage.textContent = message;
  statusMessage.className = `status-message ${kind}`;
}

function updateButtons() {
  installButton.disabled = state.busy || !state.folder || !state.executable;
  launchButton.disabled = state.busy || !state.folder || !state.executable || !state.installed;
  uninstallButton.hidden = !state.installed;
  $('#install-label').textContent = state.installed ? 'Update ANR' : 'Install ANR';
  $('#install-status').textContent = state.installed ? 'ANR INSTALLED' : state.folder ? 'READY TO INSTALL' : 'CHOOSE A GAME';
  $('#install-status').classList.toggle('is-installed', state.installed);
}

function versionText() {
  return state.release?.name || state.release?.tag || 'Latest release';
}

async function refreshRelease() {
  $('#release-title').textContent = 'Checking latest ANR release…';
  $('#release-state').textContent = 'CONNECTING';
  $('#release-subtitle').textContent = 'GitHub release packages are cached in Documents';
  try {
    state.release = await invoke('get_latest_release');
    $('#release-title').textContent = `ANR FG ${versionText()}`;
    $('#release-subtitle').textContent = state.release.cached
      ? `Saved on this PC · ${state.release.offline ? 'offline cache' : 'ready to install'}`
      : 'Latest release found · downloads on first install';
    $('#release-state').textContent = state.release.offline ? 'SAVED' : 'LATEST';
    $('#release-state').classList.toggle('offline', state.release.offline);
  } catch (error) {
    state.release = null;
    $('#release-title').textContent = 'Could not check for ANR updates';
    $('#release-subtitle').textContent = String(error);
    $('#release-state').textContent = 'OFFLINE';
    $('#release-state').classList.add('offline');
  }
}

async function loadGpus() {
  try {
    state.gpus = await invoke('get_gpus');
    gpuSelect.innerHTML = '<option value="auto">Auto — let Windows decide</option>';
    for (const gpu of state.gpus) {
      const option = document.createElement('option');
      option.value = `${gpu.kind}:${state.gpus.indexOf(gpu)}`;
      const memory = gpu.memory_mb ? ` · ${Math.round(gpu.memory_mb / 1024)} GB VRAM` : (gpu.kind === 'integrated' ? ' · Shared memory' : '');
      option.textContent = `${gpu.name} · ${gpu.kind === 'integrated' ? 'Integrated' : 'Discrete'}${memory}`;
      gpuSelect.append(option);
    }
    const selectedAdapterExists = state.gpus.some((gpu, index) => `${gpu.kind}:${index}` === state.selectedGpu);
    $('#gpu-caption').textContent = state.gpus.length
      ? `${state.gpus.length} adapter${state.gpus.length === 1 ? '' : 's'} detected · integrated and discrete`
      : 'No adapter information returned by Windows';
    if (!selectedAdapterExists && state.selectedGpu !== 'auto') state.selectedGpu = 'auto';
    gpuSelect.value = state.selectedGpu;
  } catch (error) {
    $('#gpu-caption').textContent = String(error);
  }
}

async function checkRuntime() {
  try {
    state.runtime = await invoke('get_vc_runtime_status');
    updateRuntimeWarning();
  } catch {
    $('#runtime-warning').hidden = true;
  }
}

function updateRuntimeWarning() {
  $('#runtime-warning').hidden = !state.runtime || state.runtime.installed || state.hasOptiscaler;
}

async function inspectFolder(folder) {
  state.folder = folder;
  localStorage.setItem('game-folder', folder);
  pathValue.textContent = folder || 'Select a game folder to get started';
  pathValue.title = folder;
  state.executable = '';
  state.executables = [];
  state.hasOptiscaler = false;
  state.installed = false;
  exeSelect.disabled = true;
  exeSelect.innerHTML = '<option value="">Looking for game executables…</option>';
  showMessage('Checking the selected game folder…', 'working');
  updateButtons();
  if (!folder) {
    exeSelect.innerHTML = '<option value="">Choose a game folder first</option>';
    showMessage('');
    return;
  }
  try {
    const info = await invoke('inspect_game_folder', { gameDir: folder });
    state.executables = info.executables;
    state.hasOptiscaler = info.has_optiscaler;
    state.installed = info.installed;
    updateRuntimeWarning();
    exeSelect.innerHTML = '';
    if (!info.executables.length) {
      exeSelect.innerHTML = '<option value="">No .exe found in this folder</option>';
      $('#exe-help').textContent = 'Select the folder that contains the game executable';
      showMessage('No game executable was found here. Choose the game’s main folder.', 'error');
    } else {
      for (const executable of info.executables) {
        const option = document.createElement('option');
        option.value = executable;
        option.textContent = executable;
        exeSelect.append(option);
      }
      exeSelect.disabled = false;
      state.executable = info.executables[0];
      exeSelect.value = state.executable;
      $('#exe-help').textContent = `${info.executables.length} executable${info.executables.length === 1 ? '' : 's'} found`;
      const packageType = info.has_optiscaler ? 'Add-on package · OptiScaler detected' : 'Full package · includes OptiScaler';
      showMessage(`${packageType}${info.installed ? ' · Existing ANR install found' : ''}`, info.installed ? 'success' : '');
    }
  } catch (error) {
    exeSelect.innerHTML = '<option value="">Could not inspect folder</option>';
    showMessage(String(error), 'error');
  }
  updateButtons();
}

$('#theme-toggle').addEventListener('click', () => setTheme(state.theme === 'dark' ? 'light' : 'dark'));
$('#setup-tab').addEventListener('click', () => setView('setup'));
$('#monitor-tab').addEventListener('click', () => setView('monitor'));
document.querySelectorAll('[data-metric]').forEach((checkbox) => {
  const key = checkbox.dataset.metric;
  checkbox.checked = state.monitorPreferences[key];
  checkbox.addEventListener('change', () => {
    state.monitorPreferences[key] = checkbox.checked;
    saveMonitorPreferences();
  });
});
$('#overlay-toggle').addEventListener('click', async () => {
  try {
    state.overlayVisible = !state.overlayVisible;
    await invoke('set_overlay_visible', { visible: state.overlayVisible });
    refreshMonitor();
  } catch (error) {
    state.overlayVisible = false;
    $('#monitor-message').textContent = String(error);
  }
});
$('#monitor-stop').addEventListener('click', async () => {
  try {
    await invoke('stop_monitoring');
    if (state.overlayVisible) await invoke('set_overlay_visible', { visible: false });
    state.overlayVisible = false;
    await refreshMonitor();
  } catch (error) {
    $('#monitor-message').textContent = String(error);
  }
});
$('#browse').addEventListener('click', async () => {
  try {
    const folder = await invoke('choose_game_folder');
    if (folder) await inspectFolder(folder);
  } catch (error) {
    showMessage(String(error), 'error');
  }
});
exeSelect.addEventListener('change', () => {
  state.executable = exeSelect.value;
  updateButtons();
});
gpuSelect.addEventListener('change', () => {
  state.selectedGpu = gpuSelect.value;
  localStorage.setItem('selected-gpu', state.selectedGpu);
});
$('#refresh-release').addEventListener('click', refreshRelease);
$('#runtime-download').addEventListener('click', async () => {
  try {
    await invoke('open_vc_runtime_download');
    showMessage('Microsoft’s Visual C++ x64 runtime download was opened. Install it, then return here to launch the game.', 'working');
  } catch (error) {
    showMessage(String(error), 'error');
  }
});

installButton.addEventListener('click', async () => {
  setBusy(true, `Preparing ANR FG ${versionText()}…`);
  try {
    const result = await invoke('install_anr', { gameDir: state.folder });
    state.installed = true;
    showMessage(`${result} The game’s frame generation setting must be enabled in its graphics menu.`, 'success');
    updateButtons();
  } catch (error) {
    showMessage(String(error), 'error');
  } finally {
    setBusy(false);
  }
});

launchButton.addEventListener('click', async () => {
  setBusy(true, 'Applying GPU preference and launching your game…');
  try {
    const result = await invoke('launch_game', { gameDir: state.folder, executable: state.executable, gpu: state.selectedGpu });
    showMessage(result.message, 'success');
    invoke('start_monitoring', { processId: result.processId }).then(async (message) => {
      $('#monitor-message').textContent = message;
      await refreshMonitor();
      if (state.monitorPreferences.overlayAfterLaunch && state.monitorSnapshot?.gameRunning) {
        await invoke('set_overlay_visible', { visible: true });
        state.overlayVisible = true;
        await refreshMonitor();
      }
    }).catch((error) => {
      $('#monitor-message').textContent = `Game launched, but monitoring could not start: ${error}`;
    });
  } catch (error) {
    showMessage(String(error), 'error');
  } finally {
    setBusy(false);
  }
});

uninstallButton.addEventListener('click', async () => {
  setBusy(true, 'Restoring the game’s previous files and settings…');
  try {
    const result = await invoke('uninstall_anr', { gameDir: state.folder });
    state.installed = false;
    showMessage(result, 'success');
    updateButtons();
  } catch (error) {
    showMessage(String(error), 'error');
  } finally {
    setBusy(false);
  }
});

setTheme(state.theme);
setView('setup');
saveMonitorPreferences();
updateButtons();
if (state.folder) inspectFolder(state.folder);
loadGpus();
checkRuntime();
refreshRelease();
refreshMonitor();
window.setInterval(refreshMonitor, 900);
}
