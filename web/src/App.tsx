import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { FailedFile, FinalReport, ScanCancelledError, ScannerWorkerClient, formatBytes, parseReport } from './scan';
import { filterEvidence, sortEvidence, timelineSummary } from './view';
import { CLUSTER_RADIUS_OPTIONS, DEFAULT_CLUSTER_RADIUS, clusterEvidence, clusterSummaryLabel, evidenceLabel } from './tree';
import type { TreeCluster } from './tree';
import { densityColor, peakDensity, statusFill } from './map';
import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
import type { Evidence, Report, SortKey } from './types';
import './style.css';

type QueueItem = {
  id: number;
  file: File;
  status: 'queued' | 'scanning' | 'done' | 'error';
  progress?: string;
  error?: string;
};

type Tab = 'world' | 'characters' | 'timeline';

const ACCEPTED = '.tar.zst,.tar,.db,.chunk,.fch,.fch.old,.fch.bak';
const KINDS = ['all', 'zdo_cheated', 'station_queued_cheated', 'direct_item_data', 'container_inventory', 'indexed_item_data'];
const STATUSES = ['all', 'new', 'persisted', 'removed_or_cleared', 'observed'];

export function App() {
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [tab, setTab] = useState<Tab>('world');
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState('all');
  const [status, setStatus] = useState('all');
  const [sortKey, setSortKey] = useState<SortKey>('snapshot');
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>('asc');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [progressMessage, setProgressMessage] = useState('');
  const [exports, setExports] = useState<FinalReport | null>(null);
  const [copied, setCopied] = useState('');
  const client = useRef<ScannerWorkerClient | null>(null);
  const nextId = useRef(1);
  const runGeneration = useRef(0);

  useEffect(() => () => client.current?.dispose(), []);

  const visibleEvidence = useMemo(() => {
    if (!report) return [];
    return sortEvidence(filterEvidence(report.evidence, query, kind, status), sortKey, sortDirection);
  }, [kind, query, report, sortDirection, sortKey, status]);

  const addFiles = (files: FileList | File[]) => {
    const additions = Array.from(files).map((file) => {
      const accepted = /\.(tar\.zst|tar|db|chunk|fch(?:\.(?:old|bak))?)$/i.test(file.name);
      return {
        id: nextId.current++,
        file,
        status: accepted ? 'queued' as const : 'error' as const,
        error: accepted ? undefined : 'Unsupported format; accepted .tar.zst, .tar, .db, .chunk, .fch, .fch.old, and .fch.bak.',
      };
    });
    if (additions.length) setQueue((current) => [...current, ...additions]);
  };

  const scan = async () => {
    if (busy) return;
    const pending = queue.filter((item) => item.status === 'queued');
    if (!pending.length) {
      setMessage('Add at least one accepted file first.');
      return;
    }
    const run = ++runGeneration.current;
    setBusy(true);
    setMessage('');
    setProgressMessage('Starting scanner worker.');
    const failures: FailedFile[] = [];
    let succeeded = 0;
    try {
      const scanner = client.current ??= new ScannerWorkerClient();
      for (const item of pending) {
        setQueue((current) => current.map((entry) => entry.id === item.id ? { ...entry, status: 'scanning', progress: 'starting', error: undefined } : entry));
        try {
          await scanner.scanFile(item.file, (progress) => {
            if (run !== runGeneration.current) return;
            setProgressMessage(`${item.file.name}: ${progress.message}`);
            setQueue((current) => current.map((entry) => entry.id === item.id ? { ...entry, progress: progress.message } : entry));
          }, item.id);
          succeeded += 1;
          setQueue((current) => current.map((entry) => entry.id === item.id ? { ...entry, status: 'done', progress: 'complete' } : entry));
        } catch (error) {
          if (error instanceof ScanCancelledError) throw error;
          const detail = error instanceof Error ? error.message : 'Scan failed';
          failures.push({ name: item.file.name, error: detail });
          setQueue((current) => current.map((entry) => entry.id === item.id ? { ...entry, status: 'error', error: detail, progress: undefined } : entry));
        }
      }
      if (!succeeded) {
        setMessage('No files scanned successfully.');
        return;
      }
      const priorFailures = queue
        .filter((item) => item.status === 'error' && item.error)
        .map((item) => ({ name: item.file.name, error: item.error! }));
      const result = await scanner.report([...priorFailures, ...failures]);
      setExports(result);
      setReport(parseReport(result.json));
      setProgressMessage(`Finished: ${succeeded} file(s) scanned.`);
    } catch (error) {
      if (run !== runGeneration.current) return;
      if (error instanceof ScanCancelledError) {
        setQueue((current) => current.map((entry) => entry.status === 'scanning' ? { ...entry, status: 'queued', progress: undefined } : entry));
        setMessage('Scan cancelled.');
      } else {
        setMessage(error instanceof Error ? error.message : 'Scanner worker failed to start.');
      }
    } finally {
      if (run === runGeneration.current) setBusy(false);
    }
  };

  const selectCanonical = async (index: number) => {
    if (busy || !client.current) return;
    const run = ++runGeneration.current;
    setBusy(true);
    setMessage('');
    try {
      await client.current.setCanonical(index);
      const failures = queue
        .filter((item) => item.status === 'error' && item.error)
        .map((item) => ({ name: item.file.name, error: item.error! }));
      const result = await client.current.report(failures);
      setExports(result);
      setReport(parseReport(result.json));
      setMessage('Canonical profile selected.');
    } catch (error) {
      if (run !== runGeneration.current) return;
      setMessage(error instanceof Error ? error.message : 'Could not select canonical profile.');
    } finally {
      if (run === runGeneration.current) setBusy(false);
    }
  };

  const cancel = () => {
    if (!busy) return;
    client.current?.cancel();
  };

  const reset = () => {
    runGeneration.current += 1;
    client.current?.cancel();
    client.current = null;
    setBusy(false);
    setQueue([]);
    setReport(null);
    setExports(null);
    setMessage('');
    setProgressMessage('');
    setQuery('');
    setKind('all');
    setStatus('all');
  };

  const download = (extension: 'json' | 'csv' | 'md') => {
    if (!exports) return;
    const content = extension === 'json'
      ? exports.json
      : extension === 'csv' ? exports.csv : exports.markdown;
    const url = URL.createObjectURL(new Blob([content], { type: extension === 'json' ? 'application/json' : 'text/plain' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = `valheim-cheat-radar.${extension}`;
    link.click();
    URL.revokeObjectURL(url);
  };

  const copy = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(value);
      window.setTimeout(() => setCopied(''), 1200);
    } catch {
      setMessage('Clipboard access was not available; select the coordinate text to copy it.');
    }
  };

  const onDrop = (event: DragEvent) => {
    event.preventDefault();
    if (event.dataTransfer?.files) addFiles(event.dataTransfer.files);
  };

  return (
    <div className="shell" aria-busy={busy}>
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">{progressMessage || message}</div>
      <header className="masthead">
        <div>
          <p className="eyebrow">LOCAL / READ-ONLY / WASM</p>
          <h1>Valheim Cheat Radar</h1>
          <p className="subtitle">Unofficial Valheim save audit</p>
        </div>
        <div className="privacy" role="note">
          <strong>Private by default.</strong>
          <span>Files are parsed in this browser. No backend, analytics, remote fonts, or network requests.</span>
        </div>
      </header>

      <section className="intake panel" aria-labelledby="intake-heading">
        <div className="section-heading">
          <div><p className="eyebrow">01 / INPUTS</p><h2 id="intake-heading">Drop your evidence</h2></div>
          <span className="read-only-badge">READ-ONLY</span>
        </div>
        <div className="drop-zone" onDragOver={(event) => event.preventDefault()} onDrop={onDrop}>
          <input id="file-picker" aria-label="Choose save files" type="file" accept={ACCEPTED} multiple onChange={(event) => event.currentTarget.files && addFiles(event.currentTarget.files)} />
          <label htmlFor="file-picker" className="drop-label">
            <span className="drop-mark">+</span>
            <strong>Drag backups here or choose files</strong>
            <span>One file is processed at a time so the page stays responsive.</span>
          </label>
        </div>
        <p className="format-note"><strong>Accepted:</strong> .tar.zst (worker decompressed), decompressed .tar, legacy .db, defensible v41 .chunk, and .fch/.fch.old/.fch.bak character profiles. Unknown or future versions are rejected rather than guessed. Browser limits are 128 MiB compressed input and 256 MiB decompressed tar.</p>
        {queue.length > 0 && <Queue items={queue} />}
        <div className="actions">
          <button className="button primary" type="button" onClick={scan} disabled={busy || !queue.some((item) => item.status === 'queued')}>{busy ? 'Scanning…' : 'Scan queued files'}</button>
          <button className="button" type="button" onClick={cancel} disabled={!busy}>Cancel</button>
          <button className="button" type="button" onClick={reset}>Reset</button>
          {message && <p className="inline-error" role="alert" aria-live="assertive">{message}</p>}
        </div>
      </section>

      {report && <>
        <Summary report={report} />
        {report.partial && <p className="partial-banner" role="alert">Partial report: {report.failed_files?.length ?? 0} file(s) failed. Exact errors remain in the file queue and exports.</p>}
        <nav className="tabs" aria-label="Report views">
          <TabButton id="tab-world" active={tab === 'world'} onClick={() => setTab('world')}>World Evidence <span>{report.evidence.length}</span></TabButton>
          <TabButton id="tab-characters" active={tab === 'characters'} onClick={() => setTab('characters')}>Characters <span>{report.characters.length}</span></TabButton>
          <TabButton id="tab-timeline" active={tab === 'timeline'} onClick={() => setTab('timeline')}>Timeline <span>{report.archives.length}</span></TabButton>
        </nav>
        {tab === 'world' && <WorldView report={report} id="report-world" rows={visibleEvidence} query={query} kind={kind} status={status} sortKey={sortKey} sortDirection={sortDirection} setQuery={setQuery} setKind={setKind} setStatus={setStatus} setSortKey={setSortKey} setSortDirection={setSortDirection} copy={copy} copied={copied} />}
        {tab === 'characters' && <CharacterView report={report} id="report-characters" onSelectCanonical={selectCanonical} />}
        {tab === 'timeline' && <TimelineView report={report} id="report-timeline" />}
        <section className="downloads panel" aria-labelledby="download-heading">
          <div><p className="eyebrow">EXPORT</p><h2 id="download-heading">Take the report with you</h2><p>Downloads contain parsed evidence only. Saving is not implemented in this MVP.</p></div>
          <div className="download-actions"><button className="button" type="button" onClick={() => download('json')}>JSON</button><button className="button" type="button" onClick={() => download('csv')}>CSV</button><button className="button" type="button" onClick={() => download('md')}>Markdown</button></div>
        </section>
      </>}

      <footer><strong>Unofficial fan-made tool.</strong> Valheim is a trademark of its respective owner. No game assets, logos, fonts, or official screenshots are distributed here. Save scrubbing/editing is planned, not available.</footer>
    </div>
  );
}

function Queue({ items }: { items: QueueItem[] }) {
  return <ol className="queue" aria-label="File queue">{items.map((item) => <li key={item.id} className={`queue-item ${item.status}`} aria-busy={item.status === 'scanning'}><span className="queue-dot" aria-hidden="true" /><span className="queue-name">{item.file.name}</span><span className="queue-size">{formatBytes(item.file.size)}</span><span className="queue-status" aria-live="polite">{item.error ?? item.progress ?? item.status}</span></li>)}</ol>;
}

function Summary({ report }: { report: Report }) {
  const cards = [
    ['Archives', report.summary.archive_count],
    ['ZDO records', report.summary.zdo_count.toLocaleString()],
    ['Evidence rows', report.summary.evidence_records.toLocaleString()],
    ['Characters', report.summary.character_count],
  ];
  return <section className="summary" aria-label="Scan summary">{cards.map(([label, value]) => <article className="summary-card" key={label}><span>{label}</span><strong>{value}</strong></article>)}</section>;
}

function TabButton({ id, active, children, onClick }: { id: string; active: boolean; children: ComponentChildren; onClick: () => void }) {
  return <button id={id} className={`tab ${active ? 'selected' : ''}`} type="button" aria-current={active ? 'page' : undefined} onClick={onClick}>{children}</button>;
}

type WorldProps = {
  report: Report;
  id: string;
  rows: Evidence[];
  query: string;
  kind: string;
  status: string;
  sortKey: SortKey;
  sortDirection: 'asc' | 'desc';
  setQuery: (value: string) => void;
  setKind: (value: string) => void;
  setStatus: (value: string) => void;
  setSortKey: (value: SortKey) => void;
  setSortDirection: (value: 'asc' | 'desc') => void;
  copy: (value: string) => void;
  copied: string;
};

function WorldView({ report, id, rows, query, kind, status, sortKey, sortDirection, setQuery, setKind, setStatus, setSortKey, setSortDirection, copy, copied }: WorldProps) {
  const sort = (key: SortKey) => {
    if (key === sortKey) setSortDirection(sortDirection === 'asc' ? 'desc' : 'asc');
    else { setSortKey(key); setSortDirection('asc'); }
  };
  const [mode, setMode] = useState<'table' | 'tree' | 'map'>('table');
  const [radius, setRadius] = useState(DEFAULT_CLUSTER_RADIUS);
  const clusters = useMemo(() => (mode === 'tree' || mode === 'map' ? clusterEvidence(rows, radius) : []), [mode, rows, radius]);
  return <section id={id} className="panel results" aria-labelledby="world-heading">
    <div className="section-heading"><div><p className="eyebrow">02 / WORLD EVIDENCE</p><h2 id="world-heading">Parsed findings</h2></div><span className="result-count">{rows.length} of {report.evidence.length}</span></div>
    <div className="filters"><label>Search<input type="search" value={query} onInput={(event) => setQuery(event.currentTarget.value)} placeholder="prefab, hash, coordinate, source…" /></label><label>Kind<select value={kind} onChange={(event) => setKind(event.currentTarget.value)}>{KINDS.map((value) => <option value={value} key={value}>{value === 'all' ? 'All kinds' : value}</option>)}</select></label><label>Status<select value={status} onChange={(event) => setStatus(event.currentTarget.value)}>{STATUSES.map((value) => <option value={value} key={value}>{value === 'all' ? 'All statuses' : value}</option>)}</select></label><div className="view-toggle" role="group" aria-label="Results layout"><button type="button" className={`tab ${mode === 'table' ? 'selected' : ''}`} aria-pressed={mode === 'table'} onClick={() => setMode('table')}>Table</button><button type="button" className={`tab ${mode === 'tree' ? 'selected' : ''}`} aria-pressed={mode === 'tree'} onClick={() => setMode('tree')}>By location</button><button type="button" className={`tab ${mode === 'map' ? 'selected' : ''}`} aria-pressed={mode === 'map'} onClick={() => setMode('map')}>Map</button></div>{mode === 'tree' && <label>Cluster radius<select value={radius} onChange={(event) => setRadius(Number(event.currentTarget.value))}>{CLUSTER_RADIUS_OPTIONS.map((value) => <option value={value} key={value}>{value} m</option>)}</select></label>}</div>
    {mode === 'tree' ? <TreeView clusters={clusters} copy={copy} copied={copied} /> : mode === 'map' ? <MapView report={report} rows={rows} clusters={clusters} /> : <div className="table-wrap"><table><thead><tr><SortableHead label="Status" value="status" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Snapshot" value="snapshot" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Kind" value="kind" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Owner" value="owner_prefab_name" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Item" value="item_name" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Stack" value="stack" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Coordinates" value="x" current={sortKey} direction={sortDirection} onClick={sort} /><th>Details</th></tr></thead><tbody>{rows.map((row, index) => <EvidenceRow row={row} key={`${row.snapshot}-${row.zdo_ordinal}-${index}`} copy={copy} copied={copied} />)}</tbody></table>{rows.length === 0 && <p className="empty">No evidence matches these filters.</p>}</div>}
  </section>;
}

function SortableHead({ label, value, current, direction, onClick }: { label: string; value: SortKey; current: SortKey; direction: 'asc' | 'desc'; onClick: (key: SortKey) => void }) {
  const sorted = current === value;
  return <th aria-sort={sorted ? (direction === 'asc' ? 'ascending' : 'descending') : 'none'}><button className="sort-button" type="button" onClick={() => onClick(value)}>{label}{sorted && <span aria-label={direction === 'asc' ? 'ascending' : 'descending'}>{direction === 'asc' ? ' ↑' : ' ↓'}</span>}</button></th>;
}

function EvidenceRow({ row, copy, copied }: { row: Evidence; copy: (value: string) => void; copied: string }) {
  const coordinate = `${row.position.x}, ${row.position.y}, ${row.position.z}`;
  return <tr><td><span className={`status status-${row.status}`}>{row.status}</span></td><td>{row.snapshot}</td><td><code>{row.kind}</code></td><td>{row.owner_prefab_name ?? 'Unknown'}<small>{row.owner_prefab_hash}</small></td><td>{row.item_name ?? '—'}<small>{row.item_hash ?? ''}</small></td><td>{row.stack ?? '—'}</td><td><button className="coordinate" type="button" onClick={() => copy(coordinate)} title="Copy X, Y, Z"><span>{row.position.x.toFixed(2)}, {row.position.y.toFixed(2)}, {row.position.z.toFixed(2)}</span>{copied === coordinate && <small>copied</small>}</button></td><td><details><summary>View</summary><EvidenceDetails row={row} /></details></td></tr>;
}

function EvidenceDetails({ row }: { row: Evidence }) {
  return <dl className="details"><dt>Owner / object</dt><dd>{row.owner_prefab_name ?? 'Unknown'}<small>hash {row.owner_prefab_hash}</small></dd><dt>Occurrence</dt><dd>{row.occurrence_index}</dd><dt>First seen</dt><dd>{row.first_seen_snapshot}</dd><dt>Last seen</dt><dd>{row.last_seen_snapshot}</dd><dt>Present in latest</dt><dd>{row.present_in_latest ? 'yes' : 'no'}</dd><dt>Source</dt><dd>{row.source}</dd><dt>Chunk</dt><dd>{row.chunk ?? '—'}</dd><dt>Path</dt><dd>{row.internal_path}</dd><dt>Key</dt><dd>{row.key_name} ({row.key_hash})</dd><dt>Slot</dt><dd>{row.grid ? `${row.grid.x}, ${row.grid.y}` : '—'}</dd><dt>Quality / variant</dt><dd>{row.quality ?? '—'} / {row.variant ?? '—'}</dd><dt>Crafter</dt><dd>{row.crafter_name ?? '—'}</dd><dt>World level</dt><dd>{row.world_level ?? '—'}</dd></dl>;
}

function coordinateText(position: { x: number; y: number; z: number }): string {
  return `${position.x.toFixed(2)}, ${position.y.toFixed(2)}, ${position.z.toFixed(2)}`;
}

/**
 * Location tree: cluster (nearby site) then category (what kind of thing),
 * with the same per-record details the table shows.
 */
function TreeView({ clusters, copy, copied }: { clusters: TreeCluster[]; copy: (value: string) => void; copied: string }) {
  if (!clusters.length) return <p className="empty">No evidence matches these filters.</p>;
  return <div className="tree">{clusters.map((cluster) => <details className="tree-cluster" key={cluster.id} open={cluster.id === 0}><summary><span className="tree-title">{cluster.located ? `Cluster ${cluster.id + 1}` : 'Unknown location'}</span>{cluster.centroid && <code className="tree-coord">{coordinateText(cluster.centroid)}</code>}<span className="tree-count">{clusterSummaryLabel(cluster)}</span></summary><div className="tree-groups">{cluster.groups.map((group) => <details className="tree-group" key={group.category}><summary><span className={`tree-category tree-category-${group.category}`}>{group.label}</span><span className="tree-count">{group.rows.length}</span></summary><ul className="tree-leaves">{group.rows.map((row, index) => <li key={`${row.snapshot}-${row.zdo_ordinal}-${index}`}><details className="tree-leaf"><summary><span className="tree-leaf-name">{evidenceLabel(row)}</span><code className="tree-coord">{coordinateText(row.position)}</code>{row.stack !== null && <span className="tree-chip">stack {row.stack}</span>}<span className={`status status-${row.status}`}>{row.status}</span></summary><div className="tree-leaf-body"><button className="button coordinate" type="button" onClick={() => copy(coordinateText(row.position))} title="Copy X, Y, Z">{copied === coordinateText(row.position) ? 'Copied' : 'Copy X, Y, Z'}</button><EvidenceDetails row={row} /></div></details></li>)}</ul></details>)}</div></details>)}</div>;
}

/** Popup bodies are built from DOM nodes, never HTML strings, so prefab and item
 *  names cannot inject markup into a Leaflet popup. */
function popupShell(title: string, subtitle: string): { root: HTMLElement; body: HTMLElement } {
  const root = document.createElement('div');
  root.className = 'map-popup';
  const heading = document.createElement('strong');
  heading.textContent = title;
  const sub = document.createElement('code');
  sub.textContent = subtitle;
  const body = document.createElement('div');
  root.append(heading, sub, body);
  return { root, body };
}

function appendDefinitionRows(body: HTMLElement, entries: [string, string][]): void {
  const list = document.createElement('dl');
  for (const [key, value] of entries) {
    const term = document.createElement('dt');
    term.textContent = key;
    const detail = document.createElement('dd');
    detail.textContent = value;
    list.append(term, detail);
  }
  body.appendChild(list);
}

function evidencePopup(row: Evidence): HTMLElement {
  const { root, body } = popupShell(evidenceLabel(row), coordinateText(row.position));
  appendDefinitionRows(body, [
    ['Status', row.status],
    ['Kind', row.kind],
    ['Owner / object', row.owner_prefab_name ?? 'Unknown'],
    ['Item', row.item_name ?? '—'],
    ['Stack', row.stack === null ? '—' : String(row.stack)],
    ['Chunk', row.chunk ?? '—'],
  ]);
  return root;
}

function clusterPopup(cluster: TreeCluster): HTMLElement {
  const { root, body } = popupShell(
    cluster.located ? `Cluster ${cluster.id + 1}` : 'Unknown location',
    cluster.centroid ? coordinateText(cluster.centroid) : 'no coordinates',
  );
  const summary = document.createElement('p');
  summary.className = 'map-popup-summary';
  summary.textContent = clusterSummaryLabel(cluster);
  body.appendChild(summary);
  for (const group of cluster.groups) {
    const heading = document.createElement('h4');
    heading.textContent = group.label;
    body.appendChild(heading);
    const list = document.createElement('ul');
    for (const row of group.rows.slice(0, 25)) {
      const item = document.createElement('li');
      item.textContent = `${evidenceLabel(row)} — ${coordinateText(row.position)}`;
      list.appendChild(item);
    }
    if (group.rows.length > 25) {
      const more = document.createElement('li');
      more.textContent = `…and ${group.rows.length - 25} more`;
      list.appendChild(more);
    }
    body.appendChild(list);
  }
  return root;
}

const MAP_CELL_FALLBACK = 64;
const MAP_MIN_ZOOM = -4;
const MAP_MAX_ZOOM = 4;
const MAP_CLUSTER_RADIUS: [number, number] = [9, 24];

/**
 * Leaflet on `CRS.Simple` — a flat world coordinate space, which is exactly what
 * a Valheim world is. Pan/pinch/wheel and popups come from Leaflet; the density
 * layer is a one-pixel-per-cell canvas that Leaflet scales.
 */
function MapView({ report, rows, clusters }: { report: Report; rows: Evidence[]; clusters: TreeCluster[] }) {
  const host = useRef<HTMLDivElement | null>(null);
  const map = useRef<L.Map | null>(null);
  const layers = useRef<{ density: L.ImageOverlay | null; points: L.LayerGroup | null; groups: L.LayerGroup | null }>({ density: null, points: null, groups: null });
  const fitted = useRef(false);

  const cells = useMemo(() => report.map?.cells ?? [], [report]);
  const cellMeters = report.map?.cell_meters || MAP_CELL_FALLBACK;

  useEffect(() => {
    const element = host.current;
    if (!element || map.current) return;
    const instance = L.map(element, {
      crs: L.CRS.Simple,
      minZoom: MAP_MIN_ZOOM,
      maxZoom: MAP_MAX_ZOOM,
      zoomSnap: 0.25,
      // Leaflet's animated zoom waits on a CSS transitionend event. That is
      // reliable in a normal browser but not in automated/reduced-motion
      // contexts, where zoom then silently does nothing. Zoom is a core
      // interaction, so it is taken unanimated and deterministic here.
      zoomAnimation: false,
      attributionControl: false,
    });
    map.current = instance;
    return () => {
      instance.remove();
      map.current = null;
      layers.current = { density: null, points: null, groups: null };
      fitted.current = false;
    };
  }, []);

  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    layers.current.density?.remove();
    layers.current.density = null;
    if (!cells.length) return;

    let minX = Infinity;
    let maxX = -Infinity;
    let minZ = Infinity;
    let maxZ = -Infinity;
    for (const [cx, cz] of cells) {
      minX = Math.min(minX, cx);
      maxX = Math.max(maxX, cx);
      minZ = Math.min(minZ, cz);
      maxZ = Math.max(maxZ, cz);
    }
    const canvas = document.createElement('canvas');
    canvas.width = maxX - minX + 1;
    canvas.height = maxZ - minZ + 1;
    const context = canvas.getContext('2d');
    if (!context) return;
    const peak = peakDensity(cells);
    for (const [cx, cz, count] of cells) {
      context.fillStyle = densityColor(count, peak);
      // Row 0 holds the largest Z so the image is not mirrored vertically.
      context.fillRect(cx - minX, maxZ - cz, 1, 1);
    }
    const bounds: L.LatLngBoundsLiteral = [
      [(maxZ + 1) * cellMeters, minX * cellMeters],
      [minZ * cellMeters, (maxX + 1) * cellMeters],
    ];
    layers.current.density = L.imageOverlay(canvas.toDataURL(), bounds, {
      opacity: 0.9,
      interactive: false,
    }).addTo(instance);
    if (!fitted.current) {
      instance.fitBounds(bounds, { padding: [18, 18] });
      fitted.current = true;
    }
  }, [cells, cellMeters]);

  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    layers.current.points?.remove();
    layers.current.groups?.remove();

    const points = L.layerGroup();
    for (const row of rows) {
      L.circleMarker([row.position.z, row.position.x], {
        radius: 3.5,
        color: '#0d0d0c',
        weight: 1,
        fillColor: statusFill(row.status),
        fillOpacity: 1,
      })
        .bindPopup(() => evidencePopup(row))
        .addTo(points);
    }
    points.addTo(instance);
    layers.current.points = points;

    const groups = L.layerGroup();
    const biggest = clusters.reduce((max, cluster) => Math.max(max, cluster.count), 1);
    for (const cluster of clusters) {
      if (!cluster.centroid) continue;
      const weight = Math.sqrt(cluster.count / biggest);
      L.circleMarker([cluster.centroid.z, cluster.centroid.x], {
        radius: MAP_CLUSTER_RADIUS[0] + weight * (MAP_CLUSTER_RADIUS[1] - MAP_CLUSTER_RADIUS[0]),
        color: '#ee8a48',
        weight: 2,
        fillColor: '#d36a32',
        fillOpacity: 0.3,
      })
        .bindPopup(() => clusterPopup(cluster))
        .addTo(groups);
    }
    groups.addTo(instance);
    layers.current.groups = groups;
  }, [rows, clusters]);

  if (!report.map || !cells.length) {
    return <p className="empty">This report has no world map data. Rescan a world archive to include it.</p>;
  }

  return <div className="map-panel">
    <div className="map-host" ref={host} role="application" aria-label="World map of evidence and object density" />
    <div className="map-legend">
      <p><strong>Density</strong> is the ZDO count per {cellMeters} m cell in the newest save, so it shows where the world actually has content. Terrain is not stored in a save, so this is a content map, not satellite imagery.</p>
      <p className="map-keys"><span className="map-key map-key-cluster" />cluster (click for contents) <span className="map-key map-key-new" />new <span className="map-key map-key-persisted" />persisted <span className="map-key map-key-removed" />removed / cleared</p>
      <p className="map-hint">Drag to pan · scroll or pinch to zoom · click any marker to see what is inside.</p>
    </div>
  </div>;
}

function CharacterView({ report, id, onSelectCanonical }: { report: Report; id: string; onSelectCanonical: (index: number) => void }) {  return <section id={id} className="panel" aria-labelledby="characters-heading"><div className="section-heading"><div><p className="eyebrow">03 / CHARACTERS</p><h2 id="characters-heading">Profile history</h2></div></div><p className="explanation">A single unambiguous trusted, supported profile is selected automatically. When several are plausible, choose one explicitly; backup suffixes are never automatic canonical candidates.</p><div className="table-wrap"><table><thead><tr><th>Status</th><th>Source</th><th>Profile</th><th>Canonical</th><th>Trusted</th><th>Inventory</th><th>Indicators</th></tr></thead><tbody>{report.characters.map((character, index) => { const available = character.trusted && character.supported; const selectable = available && !/\.fch\.(?:old|bak)$/i.test(character.source); return <tr key={`${character.source}-${index}`}><td><span className={`status status-${character.status}`}>{character.status}</span></td><td>{character.source}</td><td>{available ? (character.profile_name || 'Unnamed') : 'Unavailable'}<small>{available ? `v${character.profile_version ?? '—'}` : 'unavailable'}</small></td><td>{character.canonical ? 'yes' : selectable ? <button className="button" type="button" onClick={() => onSelectCanonical(index)}>Choose</button> : 'no'}</td><td>{character.trusted ? 'yes' : 'no'}</td><td>{available ? `${character.inventory_item_count ?? '—'} items / ${character.inventory_version ?? '—'}` : 'unavailable'}</td><td>{available ? (character.used_cheats || character.cheat_stat_nonzero_count || character.cheated_inventory_count || character.bypass_cheat_checks ? 'flagged' : 'none') : 'unavailable'}</td></tr>; })}</tbody></table>{report.characters.length === 0 && <p className="empty">No character profiles were added.</p>}</div></section>;
}

function TimelineView({ report, id }: { report: Report; id: string }) {
  return <section id={id} className="panel" aria-labelledby="timeline-heading"><div className="section-heading"><div><p className="eyebrow">04 / TIMELINE</p><h2 id="timeline-heading">Save-to-save context</h2></div></div><p className="explanation">{timelineSummary(report.archives.length)}</p><div className="timeline-list">{report.archives.map((archive, index) => <article key={`${archive.snapshot}-${archive.archive}`}><span>{String(index + 1).padStart(2, '0')}</span><div><h3>{archive.snapshot}</h3><p>{archive.archive} · {archive.format}</p><strong>{archive.zdo_count.toLocaleString()} ZDOs / {(archive.item_count + archive.zdo_cheated_count + archive.station_queued_cheated_count).toLocaleString()} evidence rows</strong></div></article>)}</div><div className="planned"><p className="eyebrow">PLANNED / NOT AVAILABLE</p><h3>Save scrubbing</h3><p>This MVP never edits or rewrites a save. Any future copy-only scrubber would need an explicit preview, checksum verification, and a new download.</p></div></section>;
}
