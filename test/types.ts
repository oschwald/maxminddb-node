import maxmind, {
  CityResponse,
  MODE_MMAP,
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
  opened.clearCache();
  void cacheHits;

  const tuple = opened.getWithPrefixLength('8.8.8.8');
  const prefixLength: number = tuple[1];
  void prefixLength;

  opened.getPath('8.8.8.8', ['subdivisions', 0, 'iso_code']);
  const countryPath: PathLookup<string> = opened.path<string>(['country', 'iso_code']);
  const countryCode: string | null = countryPath.get('8.8.8.8');
  void countryCode;
  opened.getMany(['8.8.8.8']);
  opened.getManyPath(['8.8.8.8'], ['country', 'iso_code']);
  opened.within('8.8.8.0/24');
  opened.networks({ skipEmptyValues: true });
  const page = opened.withinPage('8.8.8.0/24', {
    limit: 100,
    offset: 0,
    skipEmptyValues: true,
  });
  const nextOffset: number | null = page.nextOffset;
  page.records[0]?.[1]?.country?.iso_code;
  void nextOffset;
  for (const generatedPage of opened.withinPages('8.8.8.0/24', { pageSize: 100 })) {
    generatedPage.records[0]?.[1]?.country?.iso_code;
  }
  opened.networkPages({ limit: 100 }).next();

  const fromBuffer = new Reader<CityResponse>(Buffer.alloc(0));
  fromBuffer.close();

  await maxmind.open<CityResponse>('/tmp/example.mmdb', { cache: false });

  const isValid: boolean = validate('8.8.8.8');
  void isValid;
}

void checkTypes;
