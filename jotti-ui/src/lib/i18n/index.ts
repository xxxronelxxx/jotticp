import { writable, derived } from 'svelte/store';
import en from './en.json';
import ar from './ar.json';
import fr from './fr.json';
import es from './es.json';
import zhCN from './zh-CN.json';
import ru from './ru.json';

type Language = 'en' | 'ar' | 'fr' | 'es' | 'zh-CN' | 'ru';

const translations: Record<Language, typeof en> = { en, ar, fr, es, 'zh-CN': zhCN, ru };

function getStoredLang(): Language {
    if (typeof localStorage === 'undefined') return 'ru';
    return (localStorage.getItem('jotti-lang') as Language) || 'ru';
}

export const currentLang = writable<Language>(getStoredLang());
export const isRTL = derived(currentLang, $lang => $lang === 'ar');
export const t = derived(currentLang, $lang => {
    const dict = translations[$lang] || translations.ru;
    return (key: string): string => {
        const parts = key.split('.');
        let val: unknown = dict;
        for (const part of parts) {
            if (typeof val !== 'object' || val === null) return key;
            val = (val as Record<string, unknown>)[part];
        }
        return typeof val === 'string' ? val : key;
    };
});

export function setLanguage(lang: Language) {
    currentLang.set(lang);
    if (typeof localStorage !== 'undefined') {
        localStorage.setItem('jotti-lang', lang);
    }
}
