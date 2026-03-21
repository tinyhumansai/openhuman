import Navigation from '../components/Navigation';

const platforms = [
    {
        name: 'Web',
        icon: '🌐',
        description: 'Access via browser',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
    {
        name: 'iOS',
        icon: '📱',
        description: 'Download for iPhone and iPad',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
    {
        name: 'Android',
        icon: '🤖',
        description: 'Download for Android devices',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
    {
        name: 'macOS',
        icon: '💻',
        description: 'Download for Mac',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
    {
        name: 'Windows',
        icon: '🪟',
        description: 'Download for Windows',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
    {
        name: 'Linux',
        icon: '🐧',
        description: 'Download for Linux',
        downloadLink: '#',
        status: 'Coming Soon',
        available: false,
    },
];

export default function Downloads() {
    return (
        <div className="min-h-screen bg-zinc-950 text-white">
            <Navigation />
            <main className="w-full px-6 pt-24 sm:px-8 sm:pt-32 pb-16">
                <div className="mx-auto max-w-4xl">
                    <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">
                        Download for Free
                    </h1>
                    <p className="mt-4 text-lg text-zinc-400">
                        Get OpenHuman on your preferred platform. All our frontend code is open source
                        for transparency and privacy. Anyone can audit and contribute to our codebase.
                    </p>

                    {/* Platform Downloads */}
                    <div className="mt-12 grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
                        {platforms.map((platform) => (
                            <div
                                key={platform.name}
                                className="rounded-lg border border-zinc-800 bg-zinc-900/50 p-6 hover:border-zinc-700 transition-colors"
                            >
                                <div className="text-center">
                                    <div className="text-4xl mb-4">{platform.icon}</div>
                                    <h3 className="text-xl font-semibold text-white">{platform.name}</h3>
                                    <p className="mt-2 text-sm text-zinc-400">{platform.description}</p>
                                    <div className="mt-4">
                                        {platform.available ? (
                                            <a
                                                href={platform.downloadLink}
                                                className="inline-block rounded-lg bg-white px-4 py-2 text-sm font-semibold text-zinc-950 transition-colors hover:bg-zinc-200"
                                            >
                                                Download
                                            </a>
                                        ) : (
                                            <span className="inline-block rounded-lg border border-zinc-700 px-4 py-2 text-sm font-semibold text-zinc-500 cursor-not-allowed">
                                                {platform.status}
                                            </span>
                                        )}
                                    </div>
                                </div>
                            </div>
                        ))}
                    </div>

                    {/* Open Source Section */}
                    <div className="mt-16 rounded-lg border border-zinc-800 bg-zinc-900/50 p-8">
                        <h2 className="text-2xl font-semibold text-white mb-4">Open Source & Privacy</h2>
                        <p className="text-zinc-300 leading-relaxed mb-6">
                            All of our frontend code is open source under the GNU General Public License (GPL) v3.
                            This ensures complete transparency and allows you to verify our privacy practices.
                            You can review, audit, and even contribute to our codebase.
                        </p>
                        <div className="flex flex-wrap gap-4">
                            <a
                                href="https://github.com/openhumanxyz"
                                target="_blank"
                                rel="noopener noreferrer"
                                className="inline-flex items-center gap-2 rounded-lg border border-zinc-800 px-4 py-2 text-sm font-semibold text-white transition-colors hover:border-zinc-700"
                            >
                                <svg
                                    className="h-5 w-5"
                                    fill="currentColor"
                                    viewBox="0 0 24 24"
                                    aria-hidden="true"
                                >
                                    <path
                                        fillRule="evenodd"
                                        d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z"
                                        clipRule="evenodd"
                                    />
                                </svg>
                                View Source Code
                            </a>
                            <a
                                href="https://www.gnu.org/licenses/gpl-3.0.html"
                                target="_blank"
                                rel="noopener noreferrer"
                                className="inline-flex items-center gap-2 rounded-lg border border-zinc-800 px-4 py-2 text-sm font-semibold text-white transition-colors hover:border-zinc-700"
                            >
                                <svg
                                    className="h-5 w-5"
                                    fill="none"
                                    viewBox="0 0 24 24"
                                    stroke="currentColor"
                                >
                                    <path
                                        strokeLinecap="round"
                                        strokeLinejoin="round"
                                        strokeWidth={2}
                                        d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                                    />
                                </svg>
                                GNU GPL v3 License
                            </a>
                        </div>
                    </div>

                    {/* Additional Info */}
                    <div className="mt-8 text-center">
                        <p className="text-sm text-zinc-400">
                            Questions about downloads or open source?{' '}
                            <a
                                href="mailto:support@openhuman.xyz"
                                className="text-white underline hover:text-zinc-300"
                            >
                                Contact us
                            </a>
                        </p>
                    </div>
                </div>
            </main>
        </div>
    );
}
