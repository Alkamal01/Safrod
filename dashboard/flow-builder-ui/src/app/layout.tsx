import type { Metadata } from 'next';
import './globals.scss';

export const metadata: Metadata = {
  title: 'Safrod · Build your flow',
  description: 'Explore Safrod Bitcoin, Lightning, and simulated sandbox API flows.',
  icons: { icon: '/safrod-mark.svg' },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta name="theme-color" content="#111111" />
        <script
          dangerouslySetInnerHTML={{
            __html: `(function(){try{var p=localStorage.getItem('safrod-theme')||'system';var t=p==='system'?(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'):p;document.documentElement.setAttribute('data-theme',t);document.documentElement.setAttribute('data-theme-pref',p)}catch(_){}})()`,
          }}
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
