export type Position = { x: number; y: number; z: number };

export type Evidence = {
  status: string;
  first_seen_snapshot: string;
  last_seen_snapshot: string;
  present_in_latest: boolean;
  occurrence_index: number;
  archive: string;
  snapshot: string;
  source: string;
  internal_path: string;
  format: string;
  chunk: string | null;
  chunk_version: number | null;
  chunk_size: number | null;
  chunk_revision: number | null;
  zdo_ordinal: number;
  owner_prefab_hash: number;
  owner_prefab_name: string | null;
  position: Position;
  legacy_sector: { x: number; y: number } | null;
  key_hash: number;
  key_name: string;
  kind: string;
  item_hash: number | null;
  item_name: string | null;
  grid: { x: number; y: number } | null;
  quality: number | null;
  stack: number | null;
  variant: number | null;
  crafter_name: string | null;
  world_level: number | null;
  custom_data_keys: string[];
};

export type Archive = {
  archive: string;
  snapshot: string;
  format: string;
  zdo_count: number;
  decoded_item_count: number;
  item_count: number;
  direct_item_count: number;
  container_item_count: number;
  indexed_item_count: number;
  zdo_cheated_count: number;
  station_queued_cheated_count: number;
  metadata_total: number | null;
  metadata_entries: number;
  player_profiles_present: boolean;
};

export type Character = {
  status: string;
  source: string;
  canonical: boolean;
  modified_unix: number | null;
  trusted: boolean;
  supported: boolean;
  hash_valid: boolean;
  parse_error: string | null;
  profile_name: string | null;
  profile_version: number | null;
  lineage: string;
  used_cheats: boolean | null;
  cheat_stat_nonzero_count: number | null;
  known_command_hits: number | null;
  player_data_version: number | null;
  inventory_version: number | null;
  inventory_item_count: number | null;
  cheated_inventory_count: number | null;
  bypass_cheat_checks: boolean | null;
  player_data_complete: boolean | null;
  file_bytes: number;
  payload_bytes: number;
  hash_bytes: number;
};

export type WorldMap = {
  cell_meters: number;
  snapshot: string;
  /** [cellX, cellZ, zdoCount] in `cell_meters` units. */
  cells: [number, number, number][];
};

export type Report = {
  tool: string;
  schema_version: number;
  read_only: boolean;
  summary: {
    archive_count: number;
    zdo_count: number;
    decoded_item_count: number;
    item_count: number;
    direct_item_count: number;
    container_item_count: number;
    indexed_item_count: number;
    zdo_cheated_count: number;
    station_queued_cheated_count: number;
    evidence_records: number;
    character_count: number;
  };
  archives: Archive[];
  evidence: Evidence[];
  characters: Character[];
  /** Present on reports produced by a scanner build that emits world density. */
  map?: WorldMap | null;
  timeline_note: string;
  partial?: boolean;
  failed_files?: { name: string; error: string }[];
};

export type SortKey = 'status' | 'snapshot' | 'kind' | 'owner_prefab_name' | 'item_name' | 'stack' | 'x' | 'y' | 'z';
