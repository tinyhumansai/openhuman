'use client';

import { useState, useEffect } from 'react';
import TelegramLogin from '../components/TelegramLogin';
import FeedbackList from '../components/FeedbackList';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:3001';
const USE_MOCK_DATA = true; // Set to false when ready to use real API

export interface Feedback {
    _id: string;
    title: string;
    description?: string;
    status: 'planned' | 'in_progress' | 'complete';
    type: 'feature' | 'bug' | 'improvement';
    createdBy: {
        telegramId: number;
        telegramFirstName?: string;
        telegramLastName?: string;
        telegramUsername?: string;
    };
    upvotes: number;
    downvotes: number;
    userVote: 'upvote' | 'downvote' | null;
    comments: string[];
    createdAt: string;
    updatedAt: string;
}

// Mock data
const mockFeedbacks: Feedback[] = [
    {
        _id: '1',
        title: 'Summarize conversations',
        description: 'Get up to speed more quickly in any channel, direct message (DM), or thread. Whether we\'re joining a new channel, returning from time off, or catching up on missed messages.',
        status: 'planned',
        type: 'feature',
        createdBy: {
            telegramId: 123456789,
            telegramFirstName: 'John',
            telegramLastName: 'Doe',
            telegramUsername: 'johndoe',
        },
        upvotes: 5,
        downvotes: 0,
        userVote: null,
        comments: ['1', '2'],
        createdAt: new Date(Date.now() - 86400000).toISOString(),
        updatedAt: new Date(Date.now() - 86400000).toISOString(),
    },
    {
        _id: '2',
        title: 'Create a daily recap of selected chats',
        description: 'Get caught up after some time away from Telegram with a customized summary of our unread channels. With recaps, we can get automated daily summaries for selected chats.',
        status: 'planned',
        type: 'feature',
        createdBy: {
            telegramId: 987654321,
            telegramFirstName: 'Jane',
            telegramLastName: 'Smith',
            telegramUsername: 'janesmith',
        },
        upvotes: 3,
        downvotes: 1,
        userVote: null,
        comments: ['3'],
        createdAt: new Date(Date.now() - 172800000).toISOString(),
        updatedAt: new Date(Date.now() - 172800000).toISOString(),
    },
    {
        _id: '3',
        title: 'Create a macOS & Windows App',
        description: 'For all the desktop users using these platforms',
        status: 'planned',
        type: 'feature',
        createdBy: {
            telegramId: 555555555,
            telegramFirstName: 'Bob',
            telegramLastName: 'Johnson',
            telegramUsername: 'bobjohnson',
        },
        upvotes: 8,
        downvotes: 0,
        userVote: null,
        comments: [],
        createdAt: new Date(Date.now() - 259200000).toISOString(),
        updatedAt: new Date(Date.now() - 259200000).toISOString(),
    },
    {
        _id: '4',
        title: 'Fix message sync issues',
        description: 'Messages sometimes don\'t sync properly across devices',
        status: 'in_progress',
        type: 'bug',
        createdBy: {
            telegramId: 111222333,
            telegramFirstName: 'Alice',
            telegramLastName: 'Williams',
            telegramUsername: 'alicew',
        },
        upvotes: 12,
        downvotes: 0,
        userVote: null,
        comments: ['4', '5'],
        createdAt: new Date(Date.now() - 345600000).toISOString(),
        updatedAt: new Date(Date.now() - 345600000).toISOString(),
    },
    {
        _id: '5',
        title: 'Dark mode improvements',
        description: 'Better contrast and color scheme for dark mode',
        status: 'complete',
        type: 'improvement',
        createdBy: {
            telegramId: 444555666,
            telegramFirstName: 'Charlie',
            telegramLastName: 'Brown',
            telegramUsername: 'charlieb',
        },
        upvotes: 7,
        downvotes: 0,
        userVote: null,
        comments: ['6'],
        createdAt: new Date(Date.now() - 432000000).toISOString(),
        updatedAt: new Date(Date.now() - 432000000).toISOString(),
    },
];

const mockComments: Record<string, Array<{
    _id: string;
    content: string;
    user: {
        telegramId: number;
        telegramFirstName?: string;
        telegramLastName?: string;
        telegramUsername?: string;
    };
    createdAt: string;
}>> = {
    '1': [
        {
            _id: '1',
            content: 'This would be really helpful for catching up on group chats!',
            user: {
                telegramId: 999888777,
                telegramFirstName: 'David',
                telegramLastName: 'Lee',
                telegramUsername: 'davidlee',
            },
            createdAt: new Date(Date.now() - 82800000).toISOString(),
        },
    ],
    '2': [
        {
            _id: '2',
            content: 'Great idea! Would love to see this implemented.',
            user: {
                telegramId: 123456789,
                telegramFirstName: 'John',
                telegramLastName: 'Doe',
                telegramUsername: 'johndoe',
            },
            createdAt: new Date(Date.now() - 82000000).toISOString(),
        },
    ],
    '3': [
        {
            _id: '3',
            content: 'This feature would save me so much time!',
            user: {
                telegramId: 777666555,
                telegramFirstName: 'Emma',
                telegramLastName: 'Davis',
                telegramUsername: 'emmad',
            },
            createdAt: new Date(Date.now() - 170000000).toISOString(),
        },
    ],
    '4': [
        {
            _id: '4',
            content: 'I\'ve experienced this issue multiple times. Hope it gets fixed soon!',
            user: {
                telegramId: 333444555,
                telegramFirstName: 'Frank',
                telegramLastName: 'Miller',
                telegramUsername: 'frankm',
            },
            createdAt: new Date(Date.now() - 340000000).toISOString(),
        },
        {
            _id: '5',
            content: 'Same here, very annoying bug.',
            user: {
                telegramId: 666777888,
                telegramFirstName: 'Grace',
                telegramLastName: 'Wilson',
                telegramUsername: 'gracew',
            },
            createdAt: new Date(Date.now() - 338000000).toISOString(),
        },
    ],
    '6': [
        {
            _id: '6',
            content: 'The new dark mode looks amazing! Great work!',
            user: {
                telegramId: 111222333,
                telegramFirstName: 'Alice',
                telegramLastName: 'Williams',
                telegramUsername: 'alicew',
            },
            createdAt: new Date(Date.now() - 428000000).toISOString(),
        },
    ],
};

// Global mock state (in a real app, this would be in a state management solution)
let mockFeedbacksState = [...mockFeedbacks];
let mockToken: string | null = null;

// Export functions to update mock state
export const updateMockFeedback = (feedbackId: string, updates: Partial<Feedback>) => {
    const index = mockFeedbacksState.findIndex(f => f._id === feedbackId);
    if (index !== -1) {
        mockFeedbacksState[index] = { ...mockFeedbacksState[index], ...updates };
    }
};

export const getMockFeedbacks = () => mockFeedbacksState;
export const setMockFeedbacks = (feedbacks: Feedback[]) => {
    mockFeedbacksState = feedbacks;
};

export default function FeedbackPage() {
    const [feedbacks, setFeedbacks] = useState<Feedback[]>([]);
    const [loading, setLoading] = useState(true);
    const [token, setToken] = useState<string | null>(null);
    const [user, setUser] = useState<{ id: number; first_name?: string; last_name?: string; username?: string } | null>(null);
    const [showCreateForm, setShowCreateForm] = useState(false);
    const [newFeedback, setNewFeedback] = useState<{ title: string; description: string; type: 'feature' | 'bug' | 'improvement' }>({ title: '', description: '', type: 'feature' });
    const [selectedBoard, setSelectedBoard] = useState<'feature' | 'bug'>('feature');
    const [sortBy, setSortBy] = useState<'trending' | 'newest' | 'oldest'>('trending');
    const [searchQuery, setSearchQuery] = useState('');
    const [sidebarOpen, setSidebarOpen] = useState(false);

    useEffect(() => {
        // Check for stored token
        const storedToken = localStorage.getItem('telegram_token');
        if (storedToken) {
            setToken(storedToken);
            mockToken = storedToken;
            // Mock user data
            setUser({
                id: 123456789,
                first_name: 'John',
                last_name: 'Doe',
                username: 'johndoe',
            });
        }
        loadFeedbacks();
    }, []);

    const loadFeedbacks = async () => {
        try {
            if (USE_MOCK_DATA) {
                // Mock API response
                await new Promise(resolve => setTimeout(resolve, 300)); // Simulate network delay

                // Sync mock state - initialize if empty
                if (mockFeedbacksState.length === 0) {
                    mockFeedbacksState = [...mockFeedbacks];
                }

                // Use current mockFeedbacksState
                let filteredFeedbacks = [...mockFeedbacksState].filter(f => {
                    if (selectedBoard === 'bug') {
                        return f.type === 'bug';
                    }
                    return f.type === 'feature';
                });

                // Sort feedbacks
                if (sortBy === 'trending') {
                    filteredFeedbacks.sort((a, b) => (b.upvotes - b.downvotes) - (a.upvotes - a.downvotes));
                } else if (sortBy === 'newest') {
                    filteredFeedbacks.sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
                } else {
                    filteredFeedbacks.sort((a, b) => new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime());
                }

                // Filter by search query
                if (searchQuery) {
                    filteredFeedbacks = filteredFeedbacks.filter(
                        (f) =>
                            f.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
                            f.description?.toLowerCase().includes(searchQuery.toLowerCase())
                    );
                }

                setFeedbacks(filteredFeedbacks);
            } else {
                const storedToken = localStorage.getItem('telegram_token');
                const headers: HeadersInit = {};
                if (storedToken) {
                    headers['Authorization'] = `Bearer ${storedToken}`;
                }

                const params = new URLSearchParams();
                if (selectedBoard === 'bug') {
                    params.append('type', 'bug');
                } else {
                    params.append('type', 'feature');
                }

                const response = await fetch(`${API_BASE_URL}/api/feedback?${params.toString()}`, { headers });
                const data = await response.json();
                if (data.success) {
                    let sortedFeedbacks = [...data.data];

                    // Sort feedbacks
                    if (sortBy === 'trending') {
                        sortedFeedbacks.sort((a, b) => (b.upvotes - b.downvotes) - (a.upvotes - a.downvotes));
                    } else if (sortBy === 'newest') {
                        sortedFeedbacks.sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
                    } else {
                        sortedFeedbacks.sort((a, b) => new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime());
                    }

                    // Filter by search query
                    if (searchQuery) {
                        sortedFeedbacks = sortedFeedbacks.filter(
                            (f) =>
                                f.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
                                f.description?.toLowerCase().includes(searchQuery.toLowerCase())
                        );
                    }

                    setFeedbacks(sortedFeedbacks);
                }
            }
        } catch (error) {
            console.error('Failed to load feedbacks:', error);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        loadFeedbacks();
    }, [selectedBoard, sortBy, searchQuery]);

    const handleTelegramAuth = async (telegramUser: { id: number; first_name?: string; last_name?: string; username?: string; auth_date: number; hash: string }) => {
        try {
            if (USE_MOCK_DATA) {
                // Mock login response
                await new Promise(resolve => setTimeout(resolve, 300));
                const mockJwtToken = 'mock_jwt_token_' + Date.now();
                localStorage.setItem('telegram_token', mockJwtToken);
                setToken(mockJwtToken);
                mockToken = mockJwtToken;
                setUser(telegramUser);
                await loadFeedbacks();
            } else {
                const response = await fetch(`${API_BASE_URL}/telegram/login`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        telegramId: telegramUser.id,
                        telegramUser: {
                            id: telegramUser.id,
                            first_name: telegramUser.first_name,
                            last_name: telegramUser.last_name,
                            username: telegramUser.username,
                        },
                    }),
                });

                const data = await response.json();
                if (data.success && data.token) {
                    localStorage.setItem('telegram_token', data.token);
                    setToken(data.token);
                    setUser(telegramUser);
                    await loadFeedbacks();
                }
            }
        } catch (error) {
            console.error('Failed to login:', error);
        }
    };

    const handleLogout = () => {
        localStorage.removeItem('telegram_token');
        setToken(null);
        mockToken = null;
        setUser(null);
    };

    const handleCreateFeedback = async () => {
        if (!token || !newFeedback.title.trim()) return;

        try {
            if (USE_MOCK_DATA) {
                // Mock create feedback
                await new Promise(resolve => setTimeout(resolve, 300));
                const newId = String(Date.now());
                const newFeedbackItem: Feedback = {
                    _id: newId,
                    title: newFeedback.title,
                    description: newFeedback.description,
                    status: 'planned',
                    type: newFeedback.type,
                    createdBy: {
                        telegramId: user?.id || 123456789,
                        telegramFirstName: user?.first_name,
                        telegramLastName: user?.last_name,
                        telegramUsername: user?.username,
                    },
                    upvotes: 0,
                    downvotes: 0,
                    userVote: null,
                    comments: [],
                    createdAt: new Date().toISOString(),
                    updatedAt: new Date().toISOString(),
                };
                mockFeedbacksState.push(newFeedbackItem);
                setMockFeedbacks(mockFeedbacksState);
                setShowCreateForm(false);
                setNewFeedback({ title: '', description: '', type: selectedBoard === 'bug' ? ('bug' as const) : ('feature' as const) });
                await loadFeedbacks();
            } else {
                const response = await fetch(`${API_BASE_URL}/api/feedback`, {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                        'Authorization': `Bearer ${token}`,
                    },
                    body: JSON.stringify(newFeedback),
                });

                const data = await response.json();
                if (data.success) {
                    setShowCreateForm(false);
                    setNewFeedback({ title: '', description: '', type: selectedBoard === 'bug' ? ('bug' as const) : ('feature' as const) });
                    await loadFeedbacks();
                }
            }
        } catch (error) {
            console.error('Failed to create feedback:', error);
        }
    };

    if (loading) {
        return (
            <div className="min-h-screen bg-zinc-950 text-white">
                <div className="flex items-center justify-center min-h-screen">
                    <div className="text-zinc-400">Loading...</div>
                </div>
            </div>
        );
    }

    const boardTitle = selectedBoard === 'bug' ? 'Bugs' : 'Feature Requests';
    const boardDescription = selectedBoard === 'bug'
        ? "Report any bugs or issues you've encountered. We'll prioritize fixing them based on upvotes."
        : "Any feature that you'd like to request the team to build can go in over here. We prioritize requests based on upvotes.";

    return (
        <div className="min-h-screen bg-zinc-950 text-white">
            {/* Custom Header - Mobile Responsive */}
            <header className="fixed top-0 left-1/2 z-50 w-full max-w-[1280px] -translate-x-1/2 border-x border-b border-zinc-800 bg-zinc-950/80 backdrop-blur-sm">
                <div className="px-4 sm:px-6 lg:px-8">
                    <div className="flex h-14 sm:h-16 items-center justify-between">
                        <div className="flex items-center gap-3 sm:gap-8">
                            <button
                                onClick={() => setSidebarOpen(!sidebarOpen)}
                                className="lg:hidden p-2 text-zinc-400 hover:text-white"
                            >
                                <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
                                </svg>
                            </button>
                            <h1 className="text-lg sm:text-xl font-semibold text-white">OpenHuman</h1>
                            <div className="hidden sm:flex items-center gap-4">
                                <button
                                    onClick={() => window.location.href = '/feedback?view=roadmap'}
                                    className="text-sm text-zinc-400 transition-colors hover:text-white"
                                >
                                    Roadmap
                                </button>
                                <button className="text-sm font-semibold text-white">Feedback</button>
                            </div>
                        </div>
                        <div className="flex items-center gap-2 sm:gap-4">
                            {token ? (
                                <>
                                    {user && (
                                        <div className="hidden sm:block text-xs sm:text-sm text-zinc-400 truncate max-w-[100px] sm:max-w-none">
                                            {user.first_name} {user.last_name || ''} {user.username && `(@${user.username})`}
                                        </div>
                                    )}
                                    <button
                                        onClick={handleLogout}
                                        className="rounded-lg border border-zinc-800 px-3 sm:px-4 py-1.5 sm:py-2 text-xs sm:text-sm font-semibold text-white transition-colors hover:border-zinc-700"
                                    >
                                        Logout
                                    </button>
                                </>
                            ) : (
                                <TelegramLogin onAuth={handleTelegramAuth} />
                            )}
                        </div>
                    </div>
                </div>
            </header>

            <main className="w-full px-4 sm:px-6 lg:px-8 pt-20 sm:pt-24 pb-8">
                <div className="flex gap-4 lg:gap-8">
                    {/* Mobile Sidebar Overlay */}
                    {sidebarOpen && (
                        <div
                            className="fixed inset-0 z-40 bg-black/50 lg:hidden"
                            onClick={() => setSidebarOpen(false)}
                        />
                    )}

                    {/* Sidebar - Mobile Drawer / Desktop Sidebar */}
                    <aside
                        className={`fixed lg:static inset-y-0 left-0 z-50 w-64 bg-zinc-950 border-r border-zinc-800 transform transition-transform duration-300 ease-in-out lg:transform-none ${sidebarOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'
                            }`}
                        style={{ top: '56px', height: 'calc(100vh - 56px)' }}
                    >
                        <div className="h-full overflow-y-auto p-4 sm:p-6">
                            <div className="mb-6">
                                <div className="mb-4 flex items-center justify-between lg:block">
                                    <h2 className="text-xs font-semibold uppercase tracking-wider text-zinc-500">BOARDS</h2>
                                    <button
                                        onClick={() => setSidebarOpen(false)}
                                        className="lg:hidden text-zinc-400 hover:text-white"
                                    >
                                        <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                                        </svg>
                                    </button>
                                </div>
                                <div className="space-y-2">
                                    <button
                                        onClick={() => {
                                            setSelectedBoard('feature');
                                            setSidebarOpen(false);
                                        }}
                                        className={`w-full rounded-lg px-4 py-3 text-left text-sm transition-colors ${selectedBoard === 'feature'
                                            ? 'bg-zinc-800 text-white'
                                            : 'text-zinc-400 hover:bg-zinc-900 hover:text-white'
                                            }`}
                                    >
                                        Feature Requests
                                    </button>
                                    <button
                                        onClick={() => {
                                            setSelectedBoard('bug');
                                            setSidebarOpen(false);
                                        }}
                                        className={`w-full rounded-lg px-4 py-3 text-left text-sm transition-colors ${selectedBoard === 'bug'
                                            ? 'bg-zinc-800 text-white'
                                            : 'text-zinc-400 hover:bg-zinc-900 hover:text-white'
                                            }`}
                                    >
                                        Bugs
                                    </button>
                                </div>
                            </div>
                            <div className="mt-8 text-xs text-zinc-500">
                                Powered by OpenHuman
                            </div>
                        </div>
                    </aside>

                    {/* Main Content */}
                    <div className="flex-1 min-w-0">
                        <div className="mb-4 sm:mb-6">
                            <h2 className="text-xl sm:text-2xl font-semibold">{boardTitle}</h2>
                            <p className="mt-1 sm:mt-2 text-xs sm:text-sm text-zinc-400">{boardDescription}</p>
                        </div>

                        {/* Create Form */}
                        {showCreateForm ? (
                            <div className="mb-6 sm:mb-8 rounded-lg border border-zinc-800 bg-zinc-900/50 p-4 sm:p-6">
                                <div className="space-y-4">
                                    <div>
                                        <input
                                            type="text"
                                            value={newFeedback.title}
                                            onChange={(e) => setNewFeedback({ ...newFeedback, title: e.target.value })}
                                            className="w-full rounded-lg border border-zinc-800 bg-zinc-950 px-4 py-3 text-sm sm:text-base text-white placeholder-zinc-500 focus:border-zinc-700 focus:outline-none"
                                            placeholder="Short, descriptive title"
                                        />
                                    </div>
                                    <div>
                                        <label className="mb-2 block text-xs sm:text-sm text-zinc-400">Details</label>
                                        <textarea
                                            value={newFeedback.description}
                                            onChange={(e) => setNewFeedback({ ...newFeedback, description: e.target.value })}
                                            className="w-full rounded-lg border border-zinc-800 bg-zinc-950 px-4 py-3 text-sm sm:text-base text-white placeholder-zinc-500 focus:border-zinc-700 focus:outline-none"
                                            rows={4}
                                            placeholder="Any additional details..."
                                        />
                                    </div>
                                    <div className="flex items-center justify-between gap-2">
                                        <div className="flex items-center gap-2">
                                            <button className="text-zinc-400 hover:text-white p-2">
                                                <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13" />
                                                </svg>
                                            </button>
                                        </div>
                                        <div className="flex gap-2 sm:gap-3">
                                            <button
                                                onClick={() => {
                                                    setShowCreateForm(false);
                                                    setNewFeedback({ title: '', description: '', type: selectedBoard === 'bug' ? ('bug' as const) : ('feature' as const) });
                                                }}
                                                className="rounded-lg border border-zinc-800 px-3 sm:px-4 py-2 text-xs sm:text-sm font-semibold text-white transition-colors hover:border-zinc-700"
                                            >
                                                Cancel
                                            </button>
                                            <button
                                                onClick={handleCreateFeedback}
                                                disabled={!token || !newFeedback.title.trim()}
                                                className="rounded-lg bg-yellow-500 px-3 sm:px-4 py-2 text-xs sm:text-sm font-semibold text-zinc-950 transition-colors hover:bg-yellow-400 disabled:opacity-50 disabled:cursor-not-allowed"
                                            >
                                                Create Post
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        ) : (
                            token && (
                                <button
                                    onClick={() => setShowCreateForm(true)}
                                    className="mb-4 sm:mb-6 w-full rounded-lg border border-zinc-800 bg-zinc-900/50 px-4 py-3 text-left text-sm text-zinc-400 transition-colors hover:border-zinc-700 hover:text-white"
                                >
                                    + Create new {selectedBoard === 'bug' ? 'bug report' : 'feature request'}
                                </button>
                            )
                        )}

                        {/* Filters and Search - Mobile Responsive */}
                        <div className="mb-4 sm:mb-6 space-y-3 sm:space-y-0 sm:flex sm:items-center sm:justify-between">
                            <div className="flex items-center gap-2 sm:gap-4">
                                <div className="flex items-center gap-2">
                                    <span className="text-xs sm:text-sm text-zinc-400">Showing</span>
                                    <select
                                        value={sortBy}
                                        onChange={(e) => setSortBy(e.target.value as 'trending' | 'newest' | 'oldest')}
                                        className="rounded-lg border border-zinc-800 bg-zinc-950 px-2 sm:px-3 py-1.5 sm:py-1 text-xs sm:text-sm text-white focus:border-zinc-700 focus:outline-none"
                                    >
                                        <option value="trending">Trending</option>
                                        <option value="newest">Newest</option>
                                        <option value="oldest">Oldest</option>
                                    </select>
                                    <span className="text-xs sm:text-sm text-zinc-400">posts</span>
                                </div>
                            </div>
                            <div className="relative w-full sm:w-64">
                                <input
                                    type="text"
                                    value={searchQuery}
                                    onChange={(e) => setSearchQuery(e.target.value)}
                                    className="w-full rounded-lg border border-zinc-800 bg-zinc-950 px-4 py-2 pl-9 sm:pl-10 text-sm text-white placeholder-zinc-500 focus:border-zinc-700 focus:outline-none"
                                    placeholder="Search..."
                                />
                                <svg
                                    className="absolute left-2.5 sm:left-3 top-1/2 h-4 w-4 sm:h-5 sm:w-5 -translate-y-1/2 text-zinc-500"
                                    fill="none"
                                    stroke="currentColor"
                                    viewBox="0 0 24 24"
                                >
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                                </svg>
                            </div>
                        </div>

                        {/* Feedback List */}
                        <FeedbackList
                            feedbacks={feedbacks}
                            token={token}
                            onUpdate={loadFeedbacks}
                            useMockData={USE_MOCK_DATA}
                            mockComments={mockComments}
                            setFeedbacks={setFeedbacks}
                        />
                    </div>
                </div>
            </main>
        </div>
    );
}

// Export mock data for use in other components
export { mockFeedbacksState, mockComments, mockToken };
