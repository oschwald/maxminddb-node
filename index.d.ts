export type Names = {
  readonly de?: string;
  readonly en: string;
  readonly es?: string;
  readonly fr?: string;
  readonly ja?: string;
  readonly 'pt-BR'?: string;
  readonly ru?: string;
  readonly 'zh-CN'?: string;
};

export interface CityRecord {
  readonly confidence?: number;
  readonly geoname_id: number;
  readonly names: Names;
}

export interface ContinentRecord {
  readonly code: 'AF' | 'AN' | 'AS' | 'EU' | 'NA' | 'OC' | 'SA';
  readonly geoname_id: number;
  readonly names: Names;
}

export interface RegisteredCountryRecord {
  readonly geoname_id: number;
  readonly is_in_european_union?: boolean;
  readonly iso_code: string;
  readonly names: Names;
}

export interface CountryRecord extends RegisteredCountryRecord {
  readonly confidence?: number;
}

export interface LocationRecord {
  readonly accuracy_radius: number;
  readonly average_income?: number;
  readonly latitude: number;
  readonly longitude: number;
  readonly metro_code?: number;
  readonly population_density?: number;
  readonly time_zone?: string;
}

export interface PostalRecord {
  readonly code: string;
  readonly confidence?: number;
}

export interface RepresentedCountryRecord extends RegisteredCountryRecord {
  readonly type: string;
}

export interface SubdivisionsRecord {
  readonly confidence?: number;
  readonly geoname_id: number;
  readonly iso_code: string;
  readonly names: Names;
}

export interface TraitsRecord {
  readonly autonomous_system_number?: number;
  readonly autonomous_system_organization?: string;
  readonly connection_type?: string;
  readonly domain?: string;
  ip_address?: string;
  readonly is_anonymous?: boolean;
  readonly is_anonymous_proxy?: boolean;
  readonly is_anonymous_vpn?: boolean;
  readonly is_anycast?: boolean;
  readonly is_hosting_provider?: boolean;
  readonly is_legitimate_proxy?: boolean;
  readonly is_public_proxy?: boolean;
  readonly is_residential_proxy?: boolean;
  readonly is_satellite_provider?: boolean;
  readonly is_tor_exit_node?: boolean;
  readonly isp?: string;
  readonly mobile_country_code?: string;
  readonly mobile_network_code?: string;
  readonly organization?: string;
  readonly static_ip_score?: number;
  readonly user_count?: number;
  readonly user_type?: string;
}

export interface CountryResponse {
  readonly continent?: ContinentRecord;
  readonly country?: CountryRecord;
  readonly registered_country?: RegisteredCountryRecord;
  readonly represented_country?: RepresentedCountryRecord;
  readonly traits?: TraitsRecord;
}

export interface CityResponse extends CountryResponse {
  readonly city?: CityRecord;
  readonly location?: LocationRecord;
  readonly postal?: PostalRecord;
  readonly subdivisions?: SubdivisionsRecord[];
}

export interface AnonymousIPResponse {
  ip_address?: string;
  readonly is_anonymous?: boolean;
  readonly is_anonymous_proxy?: boolean;
  readonly is_anonymous_vpn?: boolean;
  readonly is_hosting_provider?: boolean;
  readonly is_public_proxy?: boolean;
  readonly is_residential_proxy?: boolean;
  readonly is_tor_exit_node?: boolean;
}

export interface AnonymousPlusResponse extends AnonymousIPResponse {
  readonly anonymizer_confidence?: number;
  readonly network_last_seen?: string;
  readonly provider_name?: string;
}

export interface AsnResponse {
  readonly autonomous_system_number: number;
  readonly autonomous_system_organization: string;
  ip_address?: string;
}

export interface ConnectionTypeResponse {
  readonly connection_type: string;
  ip_address?: string;
}

export interface DomainResponse {
  readonly domain: string;
  ip_address?: string;
}

export interface IspResponse extends AsnResponse {
  readonly isp: string;
  readonly mobile_country_code?: string;
  readonly mobile_network_code?: string;
  readonly organization: string;
}

export type Response =
  | CountryResponse
  | CityResponse
  | AnonymousIPResponse
  | AnonymousPlusResponse
  | AsnResponse
  | ConnectionTypeResponse
  | DomainResponse
  | IspResponse;

export interface Metadata {
  readonly binaryFormatMajorVersion: number;
  readonly binaryFormatMinorVersion: number;
  readonly buildEpoch: Date;
  readonly databaseType: string;
  readonly languages: string[];
  readonly description: Record<string, string>;
  readonly ipVersion: number;
  readonly nodeCount: number;
  readonly recordSize: number;
  readonly nodeByteSize: number;
  readonly searchTreeSize: number;
  readonly treeDepth: number;
}

export interface OpenOpts {
  cache?: false | {
    max: number;
  };
  watchForUpdates?: boolean;
  watchForUpdatesNonPersistent?: boolean;
  watchForUpdatesHook?: () => void;
  mode?: typeof MODE_AUTO | typeof MODE_MMAP | typeof MODE_MEMORY | typeof MODE_BUFFER;
}

export interface NetworkIterationOptions {
  includeAliasedNetworks?: boolean;
  includeNetworksWithoutData?: boolean;
  skipEmptyValues?: boolean;
}

export interface NetworkPageOptions extends NetworkIterationOptions {
  limit?: number;
  offset?: number;
}

export interface NetworkPage<T extends Response = Response> {
  records: Array<[string, T | null]>;
  nextOffset: number | null;
}

export interface CacheStats {
  readonly enabled: boolean;
  readonly size: number;
  readonly capacity: number;
  readonly hits: number;
  readonly misses: number;
  readonly inserts: number;
  readonly evictions: number;
}

export declare const MODE_AUTO: 'auto';
export declare const MODE_MMAP: 'mmap';
export declare const MODE_MEMORY: 'memory';
export declare const MODE_BUFFER: 'buffer';

export declare class Reader<T extends Response = Response> {
  constructor(database: Buffer, options?: OpenOpts);
  readonly closed: boolean;
  metadata: Metadata;
  load(database: Buffer): void;
  reload(): void;
  close(): void;
  clearCache(): void;
  cacheStats(): CacheStats;
  get(ipAddress: string): T | null;
  getPath(ipAddress: string, path: ReadonlyArray<string | number>): unknown;
  getWithPrefixLength(ipAddress: string): [T | null, number];
  getMany(ipAddresses: ReadonlyArray<string>): Array<T | null>;
  getManyPath(
    ipAddresses: ReadonlyArray<string>,
    path: ReadonlyArray<string | number>,
  ): unknown[];
  networks(options?: NetworkIterationOptions): Array<[string, T | null]>;
  within(
    cidr: string,
    options?: NetworkIterationOptions,
  ): Array<[string, T | null]>;
  networksPage(options?: NetworkPageOptions): NetworkPage<T>;
  withinPage(cidr: string, options?: NetworkPageOptions): NetworkPage<T>;
}

export declare function open<T extends Response = Response>(
  filepath: string,
  options?: OpenOpts,
): Promise<Reader<T>>;

export declare function openSync(): never;
export declare function init(): never;
export declare function validate(ipAddress: string): boolean;
export declare function nativeVersion(): string;

declare const maxmind: {
  Reader: typeof Reader;
  open: typeof open;
  openSync: typeof openSync;
  init: typeof init;
  validate: typeof validate;
  nativeVersion: typeof nativeVersion;
  MODE_AUTO: typeof MODE_AUTO;
  MODE_MMAP: typeof MODE_MMAP;
  MODE_MEMORY: typeof MODE_MEMORY;
  MODE_BUFFER: typeof MODE_BUFFER;
};

export default maxmind;
