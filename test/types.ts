import maxmind, {
  CityResponse,
  MODE_MMAP,
  NetworkIterator,
  PathLookup,
  Reader,
  validate,
} from '..';

async function checkTypes() {
  const opened = await maxmind.open<CityResponse>('/tmp/example.mmdb', {
    mode: MODE_MMAP,
    cache: { max: 1000 },
    watchForUpdates: false,
  });

  const city = opened.get('8.8.8.8');
  city?.country?.iso_code;
  const cacheHits: number = opened.cacheStats().hits;
  const lastReloadError: NodeJS.ErrnoException | null =
    opened.lastReloadError;
  opened.clearCache();
  const reloaded: boolean = await opened.reloadAsync();
  void reloaded;
  void cacheHits;
  const reloadErrorCode: string | undefined = lastReloadError?.code;
  void lastReloadError;
  void reloadErrorCode;

  const tuple = opened.getWithPrefixLength('8.8.8.8');
  const prefixLength: number = tuple[1];
  void prefixLength;

  const subdivisionIso: string | null = opened.getPath<string>(
    '8.8.8.8',
    ['subdivisions', 0, 'iso_code'],
  );
  void subdivisionIso;
  const projected = opened.getPaths<[string | null, string | null]>(
    '8.8.8.8',
    [['country', 'iso_code'], ['continent', 'code']],
  );
  const projectedCountry: string | null = projected[0];
  void projectedCountry;
  const countryPath: PathLookup<string> = opened.path<string>(['country', 'iso_code']);
  const countryCode: string | null = countryPath.get('8.8.8.8');
  countryPath.close();
  void countryCode;
  opened.getMany(['8.8.8.8']);
  const countryCodes: Array<string | null> = opened.getManyPath<string>(
    ['8.8.8.8'],
    ['country', 'iso_code'],
  );
  void countryCodes;
  const projectedRows = opened.getManyPaths<[string | null, string | null]>(
    ['8.8.8.8'],
    [['country', 'iso_code'], ['continent', 'code']],
  );
  const projectedContinent: string | null = projectedRows[0][1];
  void projectedContinent;
  for (const [_network, record] of opened.within('8.8.8.0/24')) {
    record?.country?.iso_code;
  }
  const networks: NetworkIterator<CityResponse> = opened.networks({
    pageSize: 100,
    skipEmptyValues: true,
  });
  const next = networks.next();
  next.value?.[1]?.country?.iso_code;
  const page = networks.nextPage(100);
  page[0]?.[1]?.country?.iso_code;
  for (const generatedPage of opened.withinPages('8.8.8.0/24', { pageSize: 100 })) {
    generatedPage[0]?.[1]?.country?.iso_code;
  }
  opened.networkPages({ pageSize: 100 }).next();
  opened.networksPath<string>(['country', 'iso_code']).next();
  opened.withinPath<string>('8.8.8.0/24', ['country', 'iso_code']).next();

  const NetworkIteratorCtor: typeof NetworkIterator = maxmind.NetworkIterator;
  void NetworkIteratorCtor;

  interface CustomRecord {
    readonly tenant: string;
    readonly score: number;
  }
  const custom = await maxmind.open<CustomRecord>('/tmp/custom.mmdb', {
    cache: {},
  });
  const tenant: string | undefined = custom.get('8.8.8.8')?.tenant;
  void tenant;
  custom[Symbol.dispose]();

  const fromBuffer = new Reader<CityResponse>(Buffer.alloc(0));
  fromBuffer.close();

  await maxmind.open<CityResponse>('/tmp/example.mmdb', { cache: false });

  const isValid: boolean = validate('8.8.8.8');
  void isValid;
}

void checkTypes;
