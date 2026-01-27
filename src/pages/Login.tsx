import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { openUrl } from '@tauri-apps/plugin-opener';
import { countries } from '../data/countries';
import { BACKEND_URL } from '../utils/config';

const Login = () => {
  const navigate = useNavigate();
  const [phoneNumber, setPhoneNumber] = useState('');
  const [selectedCountry] = useState(countries.find(c => c.code === 'US') || countries[0]);

  const formatPhoneNumber = (value: string) => {
    // Remove all non-digits
    const digits = value.replace(/\D/g, '');

    // Format as (XXX) XXX-XXXX for US numbers
    if (digits.length <= 10) {
      if (digits.length >= 6) {
        return `(${digits.slice(0, 3)}) ${digits.slice(3, 6)}-${digits.slice(6)}`;
      } else if (digits.length >= 3) {
        return `(${digits.slice(0, 3)}) ${digits.slice(3)}`;
      } else {
        return digits;
      }
    }
    return value;
  };

  const handlePhoneChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const formatted = formatPhoneNumber(e.target.value);
    setPhoneNumber(formatted);
  };

  const handlePhoneAuth = () => {
    if (phoneNumber.trim()) {
      navigate('/onboarding/step1');
    }
  };

  const handleTelegramAuth = async () => {
    try {
      await openUrl(`${BACKEND_URL}/auth/telegram?platform=desktop`);
    } catch (error) {
      console.error('Failed to open Telegram auth page:', error);
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-primary-200 via-sage-100 to-amber-100 relative flex items-center justify-center">
      {/* Background pattern */}
      <div className="absolute inset-0 bg-noise opacity-30"></div>

      {/* Main content */}
      <div className="relative z-10 max-w-md w-full mx-4">
        {/* Login/Signup card */}
        <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
          {/* Header */}
          <div className="text-center mb-8">
            <h1 className="text-2xl font-bold text-stone-900 mb-2">
              Join the Community
            </h1>
            <p className="text-stone-600">
              Choose your preferred method to get started
            </p>
          </div>

          {/* Phone input section */}
          <div className="space-y-4 mb-6">
            {/* Phone number input with country flag */}
            <div className="relative">
              <input
                type="tel"
                value={phoneNumber}
                onChange={handlePhoneChange}
                placeholder="(000) 000-0000"
                className="input-primary rounded-xl pl-20"
              />
              <div className="absolute left-4 top-1/2 transform -translate-y-1/2 flex items-center space-x-2 text-stone-500">
                <span className="text-lg">{selectedCountry.flag}</span>
                <span className="text-sm">{selectedCountry.dialCode}</span>
              </div>
            </div>
          </div>

          {/* Continue with phone button */}
          <button
            onClick={handlePhoneAuth}
            disabled={!phoneNumber.trim()}
            className="btn-primary w-full py-4 text-lg font-semibold rounded-xl mb-4 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Continue with phone
          </button>

          {/* Telegram authentication */}
          <button
            onClick={handleTelegramAuth}
            className="w-full flex items-center justify-center space-x-3 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white font-semibold py-4 rounded-xl transition-all duration-300 hover:shadow-medium"
          >
            <svg className="w-6 h-6" viewBox="0 0 24 24" fill="currentColor">
              <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z"/>
            </svg>
            <span>Continue with Telegram</span>
          </button>

          {/* Security note */}
          <div className="mt-6 p-4 bg-sage-50 rounded-xl border border-sage-200">
            <div className="flex items-start space-x-2">
              <svg className="w-5 h-5 text-sage-600 mt-0.5" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" d="M10 1L5 6v4c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V6l-5-5z"/>
              </svg>
              <div>
                <p className="text-sage-800 font-medium text-sm">Secure Authentication</p>
                <p className="text-sage-600 text-xs mt-1">Your data is protected with enterprise-grade encryption</p>
              </div>
            </div>
          </div>
        </div>

        {/* Back button */}
        <button
          onClick={() => navigate('/')}
          className="mt-6 w-full text-stone-500 hover:text-stone-700 text-sm font-medium transition-colors"
        >
          ← Back to welcome
        </button>
      </div>
    </div>
  );
};

export default Login;