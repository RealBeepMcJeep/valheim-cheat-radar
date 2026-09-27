import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { FailedFile, FinalReport, ScanCancelledError, ScannerWorkerClient, formatBytes, parseReport, scrubKind } from './scan';
import type { ScrubResult } from './scan';
import { SCRUB_MODES, summarizeScrub } from './scrub';
import type { ScrubAudit, ScrubMode } from './scrub';
import { filterEvidence, multiArchiveHint, sortEvidence, timelineSummary } from './view';
import { CLUSTER_RADIUS_OPTIONS, DEFAULT_CLUSTER_RADIUS, clusterEvidence, clusterSummaryLabel, evidenceLabel, spatialSort } from './tree';
import type { ClusterOrder, TreeCluster } from './tree';
import {
  biomeColor,
  biomeSourceText,
  biomeVerdictText,
  bucketByPixel,
  densityColor,
  densityGradientCss,
  densityTickCounts,
  indexBiomeDetail,
  mostSevereStatus,
  peakDensity,
  statusFill,
} from './map';
import type { CellBiomeDetail } from './map';
import L from 'leaflet';
import 'leaflet/dist/leaflet.css';
import type { Archive, Evidence, Report, SortKey } from './types';
import './style.css';

type QueueItem = {
  id: number;
  file: File;
  status: 'queued' | 'scanning' | 'done' | 'error';
  progress?: string;
  error?: string;
};

type Tab = 'world' | 'characters' | 'timeline';
type ScrubState = { status: 'running'; progress: string } | { status: 'done'; result: ScrubResult; acknowledged: boolean } | { status: 'error'; error: string };

// Revoking right after click can cancel a large download in some browsers, so revoke later.
function saveFile(content: BlobPart, name: string, type: string): void {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

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
  const [scrubMode, setScrubMode] = useState<ScrubMode>('clean');
  const [scrubs, setScrubs] = useState<Record<number, ScrubState>>({});
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

  const scrubItem = async (item: QueueItem) => {
    if (busy) return;
    const run = ++runGeneration.current;
    setBusy(true);
    setScrubs((current) => ({ ...current, [item.id]: { status: 'running', progress: 'starting' } }));
    try {
      const scanner = client.current ??= new ScannerWorkerClient();
      const result = await scanner.scrubFile(item.file, scrubMode, (progress) => {
        if (run !== runGeneration.current) return;
        setScrubs((current) => ({ ...current, [item.id]: { status: 'running', progress: progress.message } }));
      });
      setScrubs((current) => ({ ...current, [item.id]: { status: 'done', result, acknowledged: false } }));
    } catch (error) {
      if (run !== runGeneration.current) return;
      const detail = error instanceof ScanCancelledError ? 'Cancelled.' : error instanceof Error ? error.message : 'Scrub failed.';
      setScrubs((current) => ({ ...current, [item.id]: { status: 'error', error: detail } }));
    } finally {
      if (run === runGeneration.current) setBusy(false);
    }
  };

  const acknowledgeScrub = (id: number, acknowledged: boolean) => {
    setScrubs((current) => {
      const state = current[id];
      return state?.status === 'done' ? { ...current, [id]: { ...state, acknowledged } } : current;
    });
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
    setScrubs({});
    setScrubMode('clean');
  };

  const download = (extension: 'json' | 'csv' | 'md') => {
    if (!exports) return;
    const content = extension === 'json'
      ? exports.json
      : extension === 'csv' ? exports.csv : exports.markdown;
    saveFile(content, `valheim-cheat-radar.${extension}`, extension === 'json' ? 'application/json' : 'text/plain');
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
        {report && report.archives.length > 0 && <WorldDetailsPanel archive={report.archives[report.archives.length - 1]} copy={copy} copied={copied} />}
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
        {tab === 'timeline' && <TimelineView report={report} id="report-timeline" copy={copy} copied={copied} />}
        <section className="downloads panel" aria-labelledby="download-heading">
          <div><p className="eyebrow">EXPORT</p><h2 id="download-heading">Take the report with you</h2><p>Report downloads contain parsed evidence only.</p></div>
          <div className="download-actions"><button className="button" type="button" onClick={() => download('json')}>JSON</button><button className="button" type="button" onClick={() => download('csv')}>CSV</button><button className="button" type="button" onClick={() => download('md')}>Markdown</button></div>
        </section>
        {queue.some((item) => item.status === 'done' && scrubKind(item.file.name)) && <ScrubPanel items={queue.filter((item) => item.status === 'done' && scrubKind(item.file.name))} scrubs={scrubs} mode={scrubMode} setMode={setScrubMode} busy={busy} onPrepare={scrubItem} onAcknowledge={acknowledgeScrub} />}
      </>}

      <footer><strong>Unofficial fan-made tool.</strong> Valheim is a trademark of its respective owner. No game assets, logos, fonts, or official screenshots are distributed here. This page never changes your files; the experimental scrub only creates a new copy.</footer>
    </div>
  );
}

type ScrubPanelProps = {
  items: QueueItem[];
  scrubs: Record<number, ScrubState>;
  mode: ScrubMode;
  setMode: (mode: ScrubMode) => void;
  busy: boolean;
  onPrepare: (item: QueueItem) => void;
  onAcknowledge: (id: number, acknowledged: boolean) => void;
};

function ScrubPanel({ items, scrubs, mode, setMode, busy, onPrepare, onAcknowledge }: ScrubPanelProps) {
  const modeLabel = (value: ScrubMode) => SCRUB_MODES.find((entry) => entry.mode === value)?.label ?? value;
  return <section className="scrub panel" aria-labelledby="scrub-heading">
    <div className="section-heading">
      <div><p className="eyebrow">EXPERIMENTAL</p><h2 id="scrub-heading">Scrub a copy</h2></div>
      <span className="experimental-badge">EXPERIMENTAL</span>
    </div>
    <p className="explanation">Makes a new copy of a world backup and never changes your original. The copy has the original's format, so it can go back where it came from, and a <code>-scrubbed</code> suffix so the two are never mixed up. Preparing a copy shows exactly what it changed before anything can be downloaded.</p>
    <fieldset className="scrub-modes">
      <legend>What to do with flagged content</legend>
      {SCRUB_MODES.map((entry) => <label key={entry.mode}><input type="radio" name="scrub-mode" value={entry.mode} checked={mode === entry.mode} onChange={() => setMode(entry.mode)} /> <strong>{entry.label}</strong> <span>{entry.detail}</span></label>)}
    </fieldset>
    <ul className="scrub-limits">
      <li>Only flagged content is touched. Anything spawned with <code>bypasscheatchecks</code> on carries no flag.</li>
      <li>Items in players' own inventories live in their character files, not the world, and come back with them.</li>
      <li>A scrub erases the evidence. Keep the original backup and the audit log.</li>
      <li>Stop the server before restoring a copy, or it overwrites it with the world it holds in memory. Try the copy on a test or local server first.</li>
    </ul>
    <ul className="scrub-list">{items.map((item) => {
      const state = scrubs[item.id];
      const chunkOnly = /\.chunk$/i.test(item.file.name) && mode !== 'clean';
      return <li key={item.id}>
        <div className="scrub-row"><span className="queue-name">{item.file.name}</span><button className="button" type="button" disabled={busy || chunkOnly} onClick={() => onPrepare(item)}>Prepare copy: {modeLabel(mode)}</button></div>
        {chunkOnly && <p className="scrub-status">A single .chunk file supports Clean only; the other modes also update the world's chunk index, which only a whole backup carries.</p>}
        {state?.status === 'running' && <p className="scrub-status" aria-live="polite">{state.progress}</p>}
        {state?.status === 'error' && <p className="inline-error" role="alert">{state.error}</p>}
        {state?.status === 'done' && <ScrubResultView id={item.id} originalBytes={item.file.size} result={state.result} acknowledged={state.acknowledged} modeLabel={modeLabel(state.result.mode)} onAcknowledge={onAcknowledge} />}
      </li>;
    })}</ul>
  </section>;
}

type ScrubResultViewProps = {
  id: number;
  originalBytes: number;
  result: ScrubResult;
  acknowledged: boolean;
  modeLabel: string;
  onAcknowledge: (id: number, acknowledged: boolean) => void;
};

function ScrubResultView({ id, originalBytes, result, acknowledged, modeLabel, onAcknowledge }: ScrubResultViewProps) {
  const summary = useMemo(() => summarizeScrub(JSON.parse(result.auditJson) as ScrubAudit), [result]);
  const removed = result.zdoCountBefore - result.zdoCountAfter;
  return <div className="scrub-result">
    <p><strong>{modeLabel}:</strong> {result.zdoCountBefore.toLocaleString()} objects before, {result.zdoCountAfter.toLocaleString()} after{removed ? ` (${removed.toLocaleString()} removed)` : ''}. The copy was re-scanned and checked against the original before it was offered. {formatBytes(result.bytes.byteLength)} (original {formatBytes(originalBytes)}).</p>
    <ul className="scrub-summary">{summary.lines.map((line) => <li key={line.label}><strong>{line.label}: {line.count.toLocaleString()}</strong> <span>{line.examples}</span></li>)}</ul>
    {summary.warnings.length > 0 && <ul className="scrub-warnings" role="note">{summary.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
    <label className="scrub-acknowledge"><input type="checkbox" checked={acknowledged} onChange={(event) => onAcknowledge(id, event.currentTarget.checked)} /> I have kept the original backup and accept these changes.</label>
    <div className="download-actions">
      <button className="button primary" type="button" disabled={!acknowledged} onClick={() => saveFile(result.bytes, result.name, 'application/octet-stream')}>Download {result.name}</button>
      <button className="button" type="button" onClick={() => saveFile(result.auditMarkdown, `${result.name}.scrub-audit.md`, 'text/markdown')}>Audit (Markdown)</button>
      <button className="button" type="button" onClick={() => saveFile(result.auditJson, `${result.name}.scrub-audit.json`, 'application/json')}>Audit (JSON)</button>
    </div>
  </div>;
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

/** True when an archive carries any `.fwl2`/`.db2` metadata worth showing (fields are null/absent otherwise). */
function hasWorldMeta(archive: Archive): boolean {
  return archive.world_name != null || archive.world_version != null || archive.world_seed != null ||
    archive.world_player_count != null || (archive.global_keys?.length ?? 0) > 0;
}

/**
 * Compact expandable world-metadata summary for the newest snapshot. Seed is shown plainly (with a
 * copy control, same as a coordinate) rather than hidden behind the toggle; only the individual
 * progression flags wait for expansion.
 */
function WorldDetailsPanel({ archive, copy, copied }: { archive: Archive; copy: (value: string) => void; copied: string }) {
  if (!hasWorldMeta(archive) && !archive.world_metadata_error) return null;
  const flags = archive.global_keys ?? [];
  return <details className="world-details">
    <summary>
      <span className="eyebrow">World details</span>
      <span className="world-fields">
        {archive.world_name != null && <strong>{archive.world_name}</strong>}
        {archive.world_version != null && <span>v{archive.world_version}</span>}
        {archive.world_seed != null && <span className="world-seed">
          <code>{archive.world_seed}</code>
          <button className="button" type="button" onClick={(event) => { event.preventDefault(); event.stopPropagation(); copy(archive.world_seed!); }}>
            {copied === archive.world_seed ? 'Copied' : 'Copy seed'}
          </button>
        </span>}
        {archive.world_player_count != null && <span>{archive.world_player_count} player{archive.world_player_count === 1 ? '' : 's'}</span>}
        <span>{flags.length} progression flag{flags.length === 1 ? '' : 's'}</span>
      </span>
    </summary>
    <div className="world-details-body">
      {flags.length > 0 && <ul className="world-flags">{flags.map((flag) => <li key={flag.key}><code>{flag.key}</code>{flag.value != null && <span>{flag.value}</span>}</li>)}</ul>}
      {archive.world_metadata_error && <p className="inline-error" role="alert">World metadata: {archive.world_metadata_error}</p>}
    </div>
  </details>;
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
  const [clusterOrder, setClusterOrder] = useState<ClusterOrder>('size');
  const clusters = useMemo(() => (mode === 'tree' || mode === 'map' ? clusterEvidence(rows, radius) : []), [mode, rows, radius]);
  const orderedClusters = useMemo(() => (clusterOrder === 'spatial' ? spatialSort(clusters) : clusters), [clusters, clusterOrder]);
  const hint = multiArchiveHint(report.archives.length);
  return <section id={id} className="panel results" aria-labelledby="world-heading">
    <div className="section-heading"><div><p className="eyebrow">02 / WORLD EVIDENCE</p><h2 id="world-heading">Parsed findings</h2></div><span className="result-count">{rows.length} of {report.evidence.length}</span></div>
    {hint && <p className="explanation multi-archive-hint">{hint}</p>}
    <div className="filters"><label>Search<input type="search" value={query} onInput={(event) => setQuery(event.currentTarget.value)} placeholder="prefab, hash, coordinate, source…" /></label><label>Kind<select value={kind} onChange={(event) => setKind(event.currentTarget.value)}>{KINDS.map((value) => <option value={value} key={value}>{value === 'all' ? 'All kinds' : value}</option>)}</select></label><label>Status<select value={status} onChange={(event) => setStatus(event.currentTarget.value)}>{STATUSES.map((value) => <option value={value} key={value}>{value === 'all' ? 'All statuses' : value}</option>)}</select></label><div className="view-toggle" role="group" aria-label="Results layout"><button type="button" className={`tab ${mode === 'table' ? 'selected' : ''}`} aria-pressed={mode === 'table'} onClick={() => setMode('table')}>Table</button><button type="button" className={`tab ${mode === 'tree' ? 'selected' : ''}`} aria-pressed={mode === 'tree'} onClick={() => setMode('tree')}>By location</button><button type="button" className={`tab ${mode === 'map' ? 'selected' : ''}`} aria-pressed={mode === 'map'} onClick={() => setMode('map')}>Map</button></div>{mode === 'tree' && <label>Cluster radius<select value={radius} onChange={(event) => setRadius(Number(event.currentTarget.value))}>{CLUSTER_RADIUS_OPTIONS.map((value) => <option value={value} key={value}>{value} m</option>)}</select></label>}{mode === 'tree' && <label>Cluster order<select value={clusterOrder} onChange={(event) => setClusterOrder(event.currentTarget.value as ClusterOrder)}><option value="size">Largest first</option><option value="spatial">Nearby together</option></select></label>}</div>
    {mode === 'tree' ? <TreeView clusters={orderedClusters} copy={copy} copied={copied} /> : mode === 'map' ? <MapView report={report} rows={rows} clusters={clusters} /> : <div className="table-wrap"><table><thead><tr><SortableHead label="Status" value="status" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Snapshot" value="snapshot" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Kind" value="kind" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Owner" value="owner_prefab_name" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Item" value="item_name" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Stack" value="stack" current={sortKey} direction={sortDirection} onClick={sort} /><SortableHead label="Coordinates" value="x" current={sortKey} direction={sortDirection} onClick={sort} /><th>Details</th></tr></thead><tbody>{rows.map((row, index) => <EvidenceRow row={row} key={`${row.snapshot}-${row.zdo_ordinal}-${index}`} copy={copy} copied={copied} />)}</tbody></table>{rows.length === 0 && <p className="empty">No evidence matches these filters.</p>}</div>}
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
  return <div className="tree">{clusters.map((cluster) => <details className="tree-cluster" key={cluster.id} open={cluster.id === 0}><summary><span className="tree-title">{cluster.located ? `Cluster ${cluster.id + 1}` : 'Unknown location'}</span>{cluster.chunkLabel && <code className="tree-chunk" title="Dominant chunk this cluster's evidence comes from">{cluster.chunkLabel}</code>}{cluster.centroid && <code className="tree-coord">{coordinateText(cluster.centroid)}</code>}<span className="tree-count">{clusterSummaryLabel(cluster)}</span></summary><div className="tree-groups">{cluster.groups.map((group) => <details className="tree-group" key={group.category}><summary><span className={`tree-category tree-category-${group.category}`}>{group.label}</span><span className="tree-count">{group.rows.length}</span></summary><ul className="tree-leaves">{group.rows.map((row, index) => <li key={`${row.snapshot}-${row.zdo_ordinal}-${index}`}><details className="tree-leaf"><summary><span className="tree-leaf-name">{evidenceLabel(row)}</span><code className="tree-coord">{coordinateText(row.position)}</code>{row.stack !== null && <span className="tree-chip">stack {row.stack}</span>}<span className={`status status-${row.status}`}>{row.status}</span></summary><div className="tree-leaf-body"><button className="button coordinate" type="button" onClick={() => copy(coordinateText(row.position))} title="Copy X, Y, Z">{copied === coordinateText(row.position) ? 'Copied' : 'Copy X, Y, Z'}</button><EvidenceDetails row={row} /></div></details></li>)}</ul></details>)}</div></details>)}</div>;
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

function pixelBadgePopup(items: Evidence[]): HTMLElement {
  const { root, body } = popupShell('Dense stack', coordinateText(items[0].position));
  const summary = document.createElement('p');
  summary.className = 'map-popup-summary';
  summary.textContent = `${items.length} records rendered on top of each other at this zoom. Zoom in to split them apart, or see them all here.`;
  body.appendChild(summary);
  const sorted = [...items].sort((a, b) => a.status.localeCompare(b.status) || evidenceLabel(a).localeCompare(evidenceLabel(b)));
  const list = document.createElement('ul');
  for (const row of sorted.slice(0, 30)) {
    const item = document.createElement('li');
    item.textContent = `${evidenceLabel(row)} — ${coordinateText(row.position)} (${row.status})`;
    list.appendChild(item);
  }
  if (sorted.length > 30) {
    const more = document.createElement('li');
    more.textContent = `…and ${sorted.length - 30} more`;
    list.appendChild(more);
  }
  body.appendChild(list);
  return root;
}

/** Cell popup for feature 1: what backs a cell's biome fill, or the lack of one. */
function cellPopup(cx: number, cz: number, cellMeters: number, count: number, detail: CellBiomeDetail | undefined, biomeNames: string[]): HTMLElement {
  const originX = cx * cellMeters;
  const originZ = cz * cellMeters;
  const { root, body } = popupShell('Map cell', `${originX}, ${originZ} to ${originX + cellMeters}, ${originZ + cellMeters}`);
  const rows: [string, string][] = [
    ['ZDO count', count.toLocaleString()],
    ['Biome verdict', biomeVerdictText(detail, biomeNames)],
  ];
  const source = biomeSourceText(detail);
  if (source) rows.push(['Verdict source', source]);
  appendDefinitionRows(body, rows);
  if (detail && detail.top.length) {
    const heading = document.createElement('h4');
    heading.textContent = 'All biome votes (incl. flora hints)';
    body.appendChild(heading);
    const list = document.createElement('ul');
    for (const [index, weight] of detail.top) {
      const item = document.createElement('li');
      item.textContent = `${biomeNames[index] ?? `biome ${index}`}: ${weight}`;
      list.appendChild(item);
    }
    body.appendChild(list);
  }
  return root;
}

function addPixelLabel(target: L.LayerGroup, latlng: L.LatLngExpression, text: string, className: string): void {
  const label = document.createElement('span');
  label.textContent = text;
  const anchor = L.circleMarker(latlng, { radius: 0, opacity: 0, fillOpacity: 0, interactive: false });
  anchor.bindTooltip(label, { permanent: true, direction: 'right', className, interactive: false, offset: [3, 0] });
  anchor.addTo(target);
}

const GRID_STEP_METERS = 1000;

/** Metre grid every `GRID_STEP_METERS`, labelled, with the origin marked — lets a hotspot's
 *  coordinates be matched against the in-game map. */
function buildGridLayer(minXMeters: number, maxXMeters: number, minZMeters: number, maxZMeters: number): L.LayerGroup {
  const group = L.layerGroup();
  const lineStyle = { color: '#f2efe6', weight: 1, opacity: 0.16, interactive: false, dashArray: '3 7' } as const;
  const startX = Math.ceil(minXMeters / GRID_STEP_METERS) * GRID_STEP_METERS;
  for (let x = startX; x <= maxXMeters; x += GRID_STEP_METERS) {
    L.polyline([[minZMeters, x], [maxZMeters, x]], lineStyle).addTo(group);
    addPixelLabel(group, [maxZMeters, x], `x ${x.toLocaleString()} m`, 'map-grid-label');
  }
  const startZ = Math.ceil(minZMeters / GRID_STEP_METERS) * GRID_STEP_METERS;
  for (let z = startZ; z <= maxZMeters; z += GRID_STEP_METERS) {
    L.polyline([[z, minXMeters], [z, maxXMeters]], lineStyle).addTo(group);
    addPixelLabel(group, [z, minXMeters], `z ${z.toLocaleString()} m`, 'map-grid-label');
  }
  if (minXMeters <= 0 && 0 <= maxXMeters && minZMeters <= 0 && 0 <= maxZMeters) {
    L.circleMarker([0, 0], { radius: 4, color: '#f2efe6', weight: 2, fillOpacity: 0, interactive: false }).addTo(group);
    addPixelLabel(group, [0, 0], '0, 0', 'map-grid-label map-grid-origin');
  }
  return group;
}

const MAP_CELL_FALLBACK = 64;
const MAP_MIN_ZOOM = -4;
const MAP_MAX_ZOOM = 4;
const MAP_CLUSTER_RADIUS: [number, number] = [9, 24];
/** Screen-pixel grid used to declutter a dense base: markers within `MAP_BUCKET_PX` at the current
 *  zoom collapse into one badge. Pixel space (not world metres), because a base looks dense or sparse
 *  relative to the screen, not to a fixed metre radius that would over-merge when zoomed out. */
const MAP_BUCKET_PX = 42;
const MAP_BADGE_RADIUS: [number, number] = [10, 24];

/**
 * Leaflet on `CRS.Simple` — a flat world coordinate space, which is exactly what
 * a Valheim world is. Pan/pinch/wheel and popups come from Leaflet; the density
 * layer is a one-pixel-per-cell canvas that Leaflet scales.
 */
function MapView({ report, rows, clusters }: { report: Report; rows: Evidence[]; clusters: TreeCluster[] }) {
  const host = useRef<HTMLDivElement | null>(null);
  const map = useRef<L.Map | null>(null);
  const layers = useRef<{ density: L.ImageOverlay | null; biome: L.ImageOverlay | null; grid: L.LayerGroup | null; points: L.LayerGroup | null; groups: L.LayerGroup | null }>({ density: null, biome: null, grid: null, points: null, groups: null });
  const fitted = useRef(false);

  const cells = useMemo(() => report.map?.cells ?? [], [report]);
  const biomes = useMemo(() => report.map?.biomes ?? [], [report]);
  const biomeNames = useMemo(() => report.map?.biome_names ?? [], [report]);
  const biomeDetailIndex = useMemo(() => indexBiomeDetail(report.map?.biome_detail), [report]);
  const cellCountIndex = useMemo(() => {
    const index = new Map<string, number>();
    for (const [x, z, count] of cells) index.set(`${x},${z}`, count);
    return index;
  }, [cells]);
  const peak = useMemo(() => peakDensity(cells), [cells]);
  const [layerView, setLayerView] = useState<'biome' | 'density' | 'both'>('both');
  const [zoom, setZoom] = useState(0);
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
    // Native Leaflet scale bar. Our CRS.Simple lat/lng *are* world metres (see the bounds math
    // below), so the control's built-in Euclidean distance() already comes out in real metres —
    // no custom scale code needed.
    L.control.scale({ metric: true, imperial: false, maxWidth: 120 }).addTo(instance);
    instance.on('zoomend', () => setZoom(instance.getZoom()));
    map.current = instance;
    return () => {
      instance.remove();
      map.current = null;
      layers.current = { density: null, biome: null, grid: null, points: null, groups: null };
      fitted.current = false;
    };
  }, []);

  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    layers.current.density?.remove();
    layers.current.biome?.remove();
    layers.current.grid?.remove();
    layers.current.density = null;
    layers.current.biome = null;
    layers.current.grid = null;
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
    for (const [cx, cz, count] of cells) {
      context.fillStyle = densityColor(count, peak);
      // Row 0 holds the largest Z so the image is not mirrored vertically.
      context.fillRect(cx - minX, maxZ - cz, 1, 1);
    }
    const minXMeters = minX * cellMeters;
    const maxXMeters = (maxX + 1) * cellMeters;
    const minZMeters = minZ * cellMeters;
    const maxZMeters = (maxZ + 1) * cellMeters;
    const bounds: L.LatLngBoundsLiteral = [[maxZMeters, minXMeters], [minZMeters, maxXMeters]];
    // Biome layer under the density shading: one pixel per cell that holds enough evidence for a
    // verdict, so undeveloped, ocean and unexplored ground stays blank instead of being guessed.
    if (biomes.length) {
      const biomeCanvas = document.createElement('canvas');
      biomeCanvas.width = canvas.width;
      biomeCanvas.height = canvas.height;
      const biomeContext = biomeCanvas.getContext('2d');
      if (biomeContext) {
        for (const [cx, cz, index] of biomes) {
          biomeContext.fillStyle = biomeColor(index);
          biomeContext.fillRect(cx - minX, maxZ - cz, 1, 1);
        }
        layers.current.biome = L.imageOverlay(biomeCanvas.toDataURL(), bounds, {
          opacity: 0.9,
          interactive: false,
        }).addTo(instance);
      }
    }
    // Density rides *under* the biome fill in "both" mode: at full strength its ember ramp hides the
    // biome hues entirely, so it is only a highlight there.
    layers.current.density = L.imageOverlay(canvas.toDataURL(), bounds, {
      opacity: 0.4,
      interactive: false,
    }).addTo(instance);
    // Metre grid + origin marker, independent of layer view, so a hotspot's coordinates can be
    // matched against the in-game map regardless of which fill is showing.
    layers.current.grid = buildGridLayer(minXMeters, maxXMeters, minZMeters, maxZMeters).addTo(instance);
    if (!fitted.current) {
      instance.fitBounds(bounds, { padding: [18, 18] });
      fitted.current = true;
      setZoom(instance.getZoom());
    }
  }, [cells, biomes, cellMeters, peak]);

  // Switching layers only changes opacity, so it never rebuilds either canvas. "Both" keeps the
  // density as a highlight (0.4) so the biome colours underneath stay legible.
  useEffect(() => {
    layers.current.biome?.setOpacity(layerView === 'density' ? 0 : 0.9);
    layers.current.density?.setOpacity(layerView === 'biome' ? 0 : layerView === 'both' ? 0.4 : 0.9);
  }, [layerView, biomes, cells]);

  // A cell click (blank canvas, not a marker — markers set `bubblingMouseEvents: false` so they
  // never reach this) shows the ZDO count and biome verdict — or the reason there is none — for
  // whichever cell was clicked, populated or not.
  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    const handleClick = (event: L.LeafletMouseEvent) => {
      const cx = Math.floor(event.latlng.lng / cellMeters);
      const cz = Math.floor(event.latlng.lat / cellMeters);
      const key = `${cx},${cz}`;
      const count = cellCountIndex.get(key) ?? 0;
      const detail = biomeDetailIndex.get(key);
      L.popup().setLatLng(event.latlng).setContent(cellPopup(cx, cz, cellMeters, count, detail, biomeNames)).openOn(instance);
    };
    instance.on('click', handleClick);
    return () => instance.off('click', handleClick);
  }, [cellCountIndex, biomeDetailIndex, biomeNames, cellMeters]);

  // Evidence markers decluttered by screen-pixel proximity at the current zoom: a dense base
  // collapses into one count badge (severity-coloured, click for the full list) instead of dozens of
  // stacked rings, and splits back into individual markers once they are far enough apart on screen.
  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    layers.current.points?.remove();

    const points = L.layerGroup();
    const projected = rows
      .filter((row) => Number.isFinite(row.position.x) && Number.isFinite(row.position.z))
      .map((row) => {
        const point = instance.project([row.position.z, row.position.x], zoom);
        return { x: point.x, y: point.y, item: row };
      });
    const buckets = bucketByPixel(projected, MAP_BUCKET_PX);
    const biggestBucket = buckets.reduce((max, bucket) => Math.max(max, bucket.items.length), 1);
    for (const bucket of buckets) {
      const latlng = instance.unproject([bucket.x, bucket.y], zoom);
      if (bucket.items.length === 1) {
        const row = bucket.items[0];
        L.circleMarker(latlng, {
          radius: 3.5,
          color: '#0d0d0c',
          weight: 1,
          fillColor: statusFill(row.status),
          fillOpacity: 1,
          bubblingMouseEvents: false,
        })
          .bindPopup(() => evidencePopup(row))
          .addTo(points);
        continue;
      }
      const weight = Math.sqrt(bucket.items.length / biggestBucket);
      const badge = L.circleMarker(latlng, {
        radius: MAP_BADGE_RADIUS[0] + weight * (MAP_BADGE_RADIUS[1] - MAP_BADGE_RADIUS[0]),
        color: '#0d0d0c',
        weight: 1.5,
        fillColor: statusFill(mostSevereStatus(bucket.items.map((row) => row.status))),
        fillOpacity: 0.85,
        bubblingMouseEvents: false,
      }).bindPopup(() => pixelBadgePopup(bucket.items));
      const label = document.createElement('span');
      label.textContent = String(bucket.items.length);
      badge.bindTooltip(label, { permanent: true, direction: 'center', className: 'map-badge-tooltip', interactive: false });
      badge.addTo(points);
    }
    points.addTo(instance);
    layers.current.points = points;
  }, [rows, zoom]);

  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    layers.current.groups?.remove();
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
        bubblingMouseEvents: false,
      })
        .bindPopup(() => clusterPopup(cluster))
        .addTo(groups);
    }
    groups.addTo(instance);
    layers.current.groups = groups;
  }, [clusters]);

  if (!report.map || !cells.length) {
    return <p className="empty">This report has no world map data. Rescan a world archive to include it.</p>;
  }

  return <div className="map-panel">
    <div className="map-host" ref={host} role="application" aria-label="World map of evidence, object density and biome" />
    <div className="map-controls" role="group" aria-label="Map layers">
      {([['both', 'Biome + density'], ['biome', 'Biome'], ['density', 'Density']] as const).map(([value, label]) => (
        <button key={value} type="button" className={`tab ${layerView === value ? 'selected' : ''}`} aria-pressed={layerView === value} onClick={() => setLayerView(value)}>{label}</button>
      ))}
    </div>
    <div className="map-legend">
      {biomeNames.length > 0 && layerView !== 'density' && <p className="map-biomes">{biomeNames.map((name, index) => <span key={name}><i style={{ background: biomeColor(index) }} />{name} ({biomes.filter(([, , cell]) => cell === index).length})</span>)}</p>}
      {biomeNames.length > 0 && <p><strong>Biomes</strong> are inferred from what the save actually contains — creatures, structures and plants the game tags with a biome. A {cellMeters} m cell is only coloured when its objects supply at least three single-biome objects' worth of weighted evidence and a 60% share for one biome; thinner cells stay blank, so undeveloped, ocean and unexplored ground is never guessed. Click any cell (coloured or not) to see the vote counts behind it.</p>}
      {layerView !== 'biome' && cells.length > 0 && <div className="map-density-scale" aria-hidden="true">
        <div className="map-density-gradient" style={{ background: densityGradientCss() }} />
        <div className="map-density-ticks">{densityTickCounts(peak).map((count, index) => <span key={index}>{count.toLocaleString()}</span>)}</div>
      </div>}
      <p><strong>Density</strong> is the ZDO count per {cellMeters} m cell in the newest save, log-scaled from indigo (sparse) to red (the densest cell, {peak.toLocaleString()} ZDOs), so it shows where the world actually has content. Terrain is not stored in a save, so this is a content map, not satellite imagery.</p>
      <p className="map-keys"><span className="map-key map-key-cluster" />cluster (click for contents) <span className="map-key map-key-badge" />N = dense stack, click to list <span className="map-key map-key-new" />new <span className="map-key map-key-persisted" />persisted <span className="map-key map-key-removed" />removed / cleared</p>
      <p className="map-hint">Drag to pan · scroll or pinch to zoom · click any marker, badge, or cell to see what is there · dashed lines are a {GRID_STEP_METERS} m metre grid.</p>
    </div>
  </div>;
}

function CharacterView({ report, id, onSelectCanonical }: { report: Report; id: string; onSelectCanonical: (index: number) => void }) {  return <section id={id} className="panel" aria-labelledby="characters-heading"><div className="section-heading"><div><p className="eyebrow">03 / CHARACTERS</p><h2 id="characters-heading">Profile history</h2></div></div><p className="explanation">A single unambiguous trusted, supported profile is selected automatically. When several are plausible, choose one explicitly; backup suffixes are never automatic canonical candidates.</p><div className="table-wrap"><table><thead><tr><th>Status</th><th>Source</th><th>Profile</th><th>Canonical</th><th>Trusted</th><th>Inventory</th><th>Indicators</th></tr></thead><tbody>{report.characters.map((character, index) => { const available = character.trusted && character.supported; const selectable = available && !/\.fch\.(?:old|bak)$/i.test(character.source); return <tr key={`${character.source}-${index}`}><td><span className={`status status-${character.status}`}>{character.status}</span></td><td>{character.source}</td><td>{available ? (character.profile_name || 'Unnamed') : 'Unavailable'}<small>{available ? `v${character.profile_version ?? '—'}` : 'unavailable'}</small></td><td>{character.canonical ? 'yes' : selectable ? <button className="button" type="button" onClick={() => onSelectCanonical(index)}>Choose</button> : 'no'}</td><td>{character.trusted ? 'yes' : 'no'}</td><td>{available ? `${character.inventory_item_count ?? '—'} items / ${character.inventory_version ?? '—'}` : 'unavailable'}</td><td>{available ? (character.used_cheats || character.cheat_stat_nonzero_count || character.cheated_inventory_count || character.bypass_cheat_checks ? 'flagged' : 'none') : 'unavailable'}</td></tr>; })}</tbody></table>{report.characters.length === 0 && <p className="empty">No character profiles were added.</p>}</div></section>;
}

function TimelineView({ report, id, copy, copied }: { report: Report; id: string; copy: (value: string) => void; copied: string }) {
  return <section id={id} className="panel" aria-labelledby="timeline-heading"><div className="section-heading"><div><p className="eyebrow">04 / TIMELINE</p><h2 id="timeline-heading">Save-to-save context</h2></div></div><p className="explanation">{timelineSummary(report.archives.length)}</p><div className="timeline-list">{report.archives.map((archive, index) => <article key={`${archive.snapshot}-${archive.archive}`}><span>{String(index + 1).padStart(2, '0')}</span><div><h3>{archive.snapshot}</h3><p>{archive.archive} · {archive.format}</p><strong>{archive.zdo_count.toLocaleString()} ZDOs / {(archive.item_count + archive.zdo_cheated_count + archive.station_queued_cheated_count).toLocaleString()} evidence rows</strong><TimelineWorldMeta archive={archive} copy={copy} copied={copied} /></div></article>)}</div><div className="planned"><p className="eyebrow">PLANNED / NOT AVAILABLE</p><h3>Save scrubbing</h3><p>This MVP never edits or rewrites a save. Any future copy-only scrubber would need an explicit preview, checksum verification, and a new download.</p></div></section>;
}

/** Same world-metadata fields as the masthead panel, compact, per archive; no flag list here — 17 archives of them would be noise. */
function TimelineWorldMeta({ archive, copy, copied }: { archive: Archive; copy: (value: string) => void; copied: string }) {
  if (!hasWorldMeta(archive) && !archive.world_metadata_error) return null;
  const flagCount = archive.global_keys?.length ?? 0;
  return <p className="timeline-world-meta">
    {archive.world_name != null && <span>{archive.world_name}</span>}
    {archive.world_version != null && <span>v{archive.world_version}</span>}
    {archive.world_seed != null && <span className="world-seed">
      <code>{archive.world_seed}</code>
      <button className="button" type="button" onClick={() => copy(archive.world_seed!)}>{copied === archive.world_seed ? 'Copied' : 'Copy seed'}</button>
    </span>}
    {archive.world_player_count != null && <span>{archive.world_player_count} player{archive.world_player_count === 1 ? '' : 's'}</span>}
    {flagCount > 0 && <span>{flagCount} progression flag{flagCount === 1 ? '' : 's'}</span>}
    {archive.world_metadata_error && <span className="inline-error" role="alert">World metadata: {archive.world_metadata_error}</span>}
  </p>;
}
