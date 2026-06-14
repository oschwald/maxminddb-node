import maxmind, {
  CityResponse,
  MODE_MMAP,
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

  const tuple = opened.getWithPrefixLength('8.8.8.8');
  const prefixLength: number = tuple[1];
  void prefixLength;

  opened.getPath('8.8.8.8', ['subdivisions', 0, 'iso_code']);
  opened.getMany(['8.8.8.8']);
  opened.getManyPath(['8.8.8.8'], ['country', 'iso_code']);
  opened.within('8.8.8.0/24');
  opened.networks({ skipEmptyValues: true });

  const fromBuffer = new Reader<CityResponse>(Buffer.alloc(0));
  fromBuffer.close();

  const isValid: boolean = validate('8.8.8.8');
  void isValid;
}

void checkTypes;

