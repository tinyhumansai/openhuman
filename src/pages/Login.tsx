import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';

import { consumeLoginToken } from '../services/api/authApi';
import { setToken } from '../store/authSlice';
import { useAppDispatch } from '../store/hooks';

const Login = () => {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const dispatch = useAppDispatch();
  const [consumeError, setConsumeError] = useState<string | null>(null);

  // Handle login token from URL (e.g. from Telegram bot "Open AlphaHuman" button)
  // Consume the token with the backend and store the returned JWT
  useEffect(() => {
    const loginToken = searchParams.get('token');
    if (!loginToken) return;

    let cancelled = false;

    (async () => {
      setConsumeError(null);
      try {
        const jwtToken = await consumeLoginToken(loginToken);
        if (cancelled) return;

        dispatch(setToken(jwtToken));
        navigate('/onboarding/', { replace: true });
      } catch (err) {
        if (!cancelled) {
          setConsumeError(err instanceof Error ? err.message : 'Login failed');
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [searchParams, dispatch, navigate]);

  if (consumeError) {
    return (
      <div className="min-h-screen relative flex items-center justify-center">
        <div className="relative z-10 max-w-md w-full mx-4 text-center">
          <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
            <p className="opacity-90 text-coral mb-4">{consumeError}</p>
            <p className="text-sm opacity-70">
              Get a new link by sending '/start login' to the AlphaHuman bot on Telegram.
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen relative flex items-center justify-center">
      <div className="relative z-10 max-w-md w-full mx-4 text-center">
        <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-white mx-auto mb-4"></div>
          <p className="opacity-70">Completing login...</p>
        </div>
      </div>
    </div>
  );
};

export default Login;
