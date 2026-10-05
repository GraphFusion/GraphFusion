// Shared by Starlight documentation and the standalone Playground page.
export const analyticsHead = [
  {
    tag: 'script',
    attrs: {
      async: true,
      src: 'https://www.googletagmanager.com/gtag/js?id=G-B1EZCM3CD7',
    },
  },
  {
    tag: 'script',
    content: `
      window.dataLayer = window.dataLayer || [];
      function gtag(){dataLayer.push(arguments);}
      if (window.location.hostname === 'graphfusion.github.io') {
        gtag('js', new Date());
        gtag('config', 'G-B1EZCM3CD7');
      }
    `,
  },
];
