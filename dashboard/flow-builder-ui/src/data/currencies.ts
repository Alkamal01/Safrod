export interface ExamplePerson { fullName: string; nationality: string }
export interface FiatCurrency {
  code: string;
  name: string;
  countryCode: string;
  accountType: string;
  accountLabel: string;
  instantRails: string[];
  allRails: string[];
  examplePerson: ExamplePerson;
}

export function railDisplayName(rail: string): string { return rail.replace(/_/g, ' ') }

// These currencies are only inputs to the explicit sandbox simulator.
export const currencies: FiatCurrency[] = [
  { code: 'NGN', name: 'Nigerian naira (simulated)', countryCode: 'ng', accountType: 'SANDBOX_BALANCE', accountLabel: 'Sandbox balance · simulated', instantRails: [], allRails: [], examplePerson: { fullName: 'Safrod Demo', nationality: 'NG' } },
  { code: 'KES', name: 'Kenyan shilling (simulated)', countryCode: 'ke', accountType: 'SANDBOX_BALANCE', accountLabel: 'Sandbox balance · simulated', instantRails: [], allRails: [], examplePerson: { fullName: 'Safrod Demo', nationality: 'KE' } },
];
